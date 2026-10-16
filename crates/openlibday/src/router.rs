//! Opening a host's port on the home router (multiplayer MP4,
//! `docs/multiplayer.md`), so players on the internet can join without
//! manual port forwarding: UPnP IGD first, then NAT-PMP. The mapping is
//! leased for an hour, renewed while the game is hosted and removed when
//! it is no longer (or expires on its own if the game crashes).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use igd_next::{AddPortError, PortMappingProtocol, SearchOptions};

/// How long a mapping lasts unless renewed, and how often it is renewed.
const LEASE: Duration = Duration::from_secs(3600);
const RENEW: Duration = Duration::from_secs(1200);
/// How long to look for each kind of router.
const SEARCH: Duration = Duration::from_secs(3);
/// The mapping's name in the router's tables.
const DESCRIPTION: &str = "openlibday";

/// How the router was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Upnp,
    NatPmp,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::Upnp => "UPnP",
            Method::NatPmp => "NAT-PMP",
        }
    }
}

/// Where players on the internet reach the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wan {
    /// Still asking the router.
    Searching,
    /// The router forwards `address` to the host. `private`: the router's
    /// own internet address is a private one, so another NAT (often the
    /// provider's) stands in the way and the address only works behind it.
    Open {
        address: SocketAddr,
        method: Method,
        private: bool,
    },
    /// No router opened the port.
    Failed(String),
}

/// A port mapping kept by a thread of its own.
pub struct PortMap {
    pub wan: Wan,
    rx: Receiver<Wan>,
    /// Dropped to tell the thread to remove the mapping and finish.
    stop: Option<Sender<()>>,
    /// Set once the mapping exists (then dropping waits for its removal).
    mapped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PortMap {
    /// Asks the router to forward TCP `port` to `port` of `lan`, this
    /// computer.
    pub fn open(port: u16, lan: Ipv4Addr) -> Self {
        let (tx, rx) = channel();
        let (stop, stopped) = channel();
        let mapped = Arc::new(AtomicBool::new(false));
        let flag = mapped.clone();
        let thread = std::thread::spawn(move || keep(port, lan, &tx, &stopped, &flag));
        Self {
            wan: Wan::Searching,
            rx,
            stop: Some(stop),
            mapped,
            thread: Some(thread),
        }
    }

    /// Takes the thread's news; whether [`PortMap::wan`] changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(wan) = self.rx.try_recv() {
            self.wan = wan;
            changed = true;
        }
        changed
    }
}

impl Drop for PortMap {
    fn drop(&mut self) {
        self.stop = None;
        // Waiting is quick once mapped; a search still running is left to
        // finish (and remove what it maps) on its own.
        if self.mapped.load(Ordering::Acquire)
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

/// A mapping made on the router.
enum Mapping {
    Upnp {
        gateway: igd_next::Gateway,
        external: u16,
        local: SocketAddr,
        lease: u32,
    },
    NatPmp {
        client: natpmp::Natpmp,
        port: u16,
    },
}

impl Mapping {
    fn renew(&mut self) -> Result<(), String> {
        match self {
            Mapping::Upnp {
                gateway,
                external,
                local,
                lease,
            } => gateway
                .add_port(
                    PortMappingProtocol::TCP,
                    *external,
                    *local,
                    *lease,
                    DESCRIPTION,
                )
                .map_err(|e| e.to_string()),
            Mapping::NatPmp { client, port } => {
                natpmp_map(client, *port, LEASE.as_secs() as u32).map(|_| ())
            }
        }
    }

    fn remove(&mut self) {
        match self {
            Mapping::Upnp {
                gateway, external, ..
            } => {
                let _ = gateway.remove_port(PortMappingProtocol::TCP, *external);
            }
            Mapping::NatPmp { client, port } => {
                let _ = natpmp_map(client, *port, 0);
            }
        }
    }
}

/// The thread: maps the port, reports, renews until stopped, removes.
fn keep(port: u16, lan: Ipv4Addr, tx: &Sender<Wan>, stop: &Receiver<()>, mapped: &AtomicBool) {
    let (mut mapping, wan) = match upnp(port, lan) {
        Ok(found) => found,
        Err(upnp_error) => match natpmp_open(port) {
            Ok(found) => found,
            Err(pmp_error) => {
                let _ = tx.send(Wan::Failed(format!(
                    "UPnP: {upnp_error}; NAT-PMP: {pmp_error}"
                )));
                return;
            }
        },
    };
    mapped.store(true, Ordering::Release);
    // Gone already (the game was left while searching): undo at once.
    if tx.send(wan.clone()).is_err() {
        mapping.remove();
        return;
    }
    // Until stopped (or the game is gone).
    let mut lost = false;
    while let Err(RecvTimeoutError::Timeout) = stop.recv_timeout(RENEW) {
        match mapping.renew() {
            Ok(()) if lost => {
                lost = false;
                let _ = tx.send(wan.clone());
            }
            Ok(()) => {}
            Err(e) => {
                lost = true;
                let _ = tx.send(Wan::Failed(format!("renewing the mapping: {e}")));
            }
        }
    }
    mapping.remove();
}

/// Whether internet players cannot reach `ip` directly: private,
/// shared (carrier-grade NAT), loopback or link-local.
fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

fn upnp(port: u16, lan: Ipv4Addr) -> Result<(Mapping, Wan), String> {
    let mut options = SearchOptions::default();
    options.bind_addr = SocketAddr::new(IpAddr::V4(lan), 0);
    options.timeout = Some(SEARCH);
    let gateway = igd_next::search_gateway(options).map_err(|e| e.to_string())?;
    let ip = gateway.get_external_ip().map_err(|e| e.to_string())?;
    let local = SocketAddr::new(IpAddr::V4(lan), port);
    let mut lease = LEASE.as_secs() as u32;
    let add = |lease| gateway.add_port(PortMappingProtocol::TCP, port, local, lease, DESCRIPTION);
    let mut result = add(lease);
    if matches!(result, Err(AddPortError::OnlyPermanentLeasesSupported)) {
        lease = 0;
        result = add(lease);
    }
    // Another computer holds the port: take whichever the router offers.
    let external = match result {
        Ok(()) => port,
        Err(AddPortError::PortInUse) => gateway
            .add_any_port(PortMappingProtocol::TCP, local, lease, DESCRIPTION)
            .map_err(|e| e.to_string())?,
        Err(e) => return Err(e.to_string()),
    };
    let wan = Wan::Open {
        address: SocketAddr::new(ip, external),
        method: Method::Upnp,
        private: private(ip),
    };
    Ok((
        Mapping::Upnp {
            gateway,
            external,
            local,
            lease,
        },
        wan,
    ))
}

/// Waits (up to [`SEARCH`]) for the answer to the request just sent.
fn natpmp_answer(client: &mut natpmp::Natpmp) -> Result<natpmp::Response, String> {
    let deadline = Instant::now() + SEARCH;
    loop {
        match client.read_response_or_retry() {
            Ok(response) => return Ok(response),
            Err(natpmp::Error::NATPMP_TRYAGAIN) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(natpmp::Error::NATPMP_TRYAGAIN) => return Err("no answer".into()),
            Err(e) => return Err(format!("{e:?}")),
        }
    }
}

/// Maps TCP `port` for `lifetime` seconds (0 removes it); the public port.
fn natpmp_map(client: &mut natpmp::Natpmp, port: u16, lifetime: u32) -> Result<u16, String> {
    client
        .send_port_mapping_request(natpmp::Protocol::TCP, port, port, lifetime)
        .map_err(|e| format!("{e:?}"))?;
    match natpmp_answer(client)? {
        natpmp::Response::TCP(m) => Ok(m.public_port()),
        other => Err(format!("unexpected answer {other:?}")),
    }
}

fn natpmp_open(port: u16) -> Result<(Mapping, Wan), String> {
    let mut client = natpmp::Natpmp::new().map_err(|e| format!("{e:?}"))?;
    client
        .send_public_address_request()
        .map_err(|e| format!("{e:?}"))?;
    let ip = match natpmp_answer(&mut client)? {
        natpmp::Response::Gateway(g) => *g.public_address(),
        other => return Err(format!("unexpected answer {other:?}")),
    };
    let external = natpmp_map(&mut client, port, LEASE.as_secs() as u32)?;
    let wan = Wan::Open {
        address: SocketAddr::new(IpAddr::V4(ip), external),
        method: Method::NatPmp,
        private: private(IpAddr::V4(ip)),
    };
    Ok((Mapping::NatPmp { client, port }, wan))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_addresses() {
        for ip in [
            "192.168.1.2",
            "10.0.0.1",
            "172.16.5.5",
            "100.64.0.1",
            "127.0.0.1",
        ] {
            assert!(private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["81.2.69.160", "100.128.0.1", "2001:db8::1"] {
            assert!(!private(ip.parse().unwrap()), "{ip}");
        }
    }
}
