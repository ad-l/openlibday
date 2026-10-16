//! The connection between a host and its clients (multiplayer MP1 and
//! MP2, `docs/multiplayer.md`). Messages travel as JSON text, in process
//! over a loopback link or between computers over TCP, each message a
//! frame of its length (4 bytes, little-endian) then its text.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::command::{Command, Entry};

/// A client's message to the host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum ToHost {
    /// The client's first message: the protocol it speaks, a digest of its
    /// game data ([`data_digest`]) and its player's name.
    Hello {
        version: u32,
        data: String,
        name: String,
    },
    /// An order for the host to validate, seed and run for the client's
    /// player.
    Request { command: Command },
    /// A chat line for every player.
    Chat { text: String },
}

/// The host's message to a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum ToClient {
    /// The client is in, playing `slot` (again when slots are renumbered
    /// after someone leaves the lobby).
    Welcome { slot: i64 },
    /// The client may not join; the link closes.
    Refuse { reason: String },
    /// The players' names by slot.
    Roster { players: Vec<(i64, String)> },
    /// A chat line from `slot` (none: a notice from the game).
    Chat { slot: Option<i64>, text: String },
    /// A skirmish begins on `map` with `humans` humans; the client plays
    /// `slot`.
    Start {
        map: String,
        humans: usize,
        slot: i64,
    },
    /// A command the host ran, for the client to run in turn.
    Run { entry: Entry },
    /// The game's settings, as the host chose them.
    Settings { settings: crate::lobby::Settings },
}

/// One direction-typed end of a link.
pub struct End<In, Out> {
    rx: Receiver<String>,
    tx: Sender<String>,
    _types: std::marker::PhantomData<(In, Out)>,
}

impl<In: for<'de> Deserialize<'de>, Out: Serialize> End<In, Out> {
    /// Sends `message`; whether the other end is still there.
    pub fn send(&self, message: &Out) -> bool {
        serde_json::to_string(message).is_ok_and(|text| self.tx.send(text).is_ok())
    }

    /// The next message received, if any (`Err` once the other end is gone
    /// or sent something unreadable).
    pub fn receive(&self) -> Result<Option<In>, String> {
        match self.rx.try_recv() {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| format!("bad message {text}: {e}")),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("disconnected".into()),
        }
    }
}

/// The host's end of a client's link.
pub type ClientLink = End<ToHost, ToClient>;
/// The client's end of its link to the host.
pub type HostLink = End<ToClient, ToHost>;

/// A host's connection to the client playing `slot`.
pub struct Peer {
    pub slot: i64,
    pub link: ClientLink,
}

/// Version of the messages above; both ends must speak the same.
pub const PROTOCOL: u32 = 4;
/// The port a host listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 41530;
/// Longest message accepted (a corrupt or hostile length otherwise
/// allocates without bound).
const MAX_FRAME: usize = 16 << 20;
/// How long joining waits for the host to answer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// An idle link sends an empty frame this often, so routers keep the
/// connection open; one silent for [`SILENCE`] is taken as broken.
const KEEPALIVE: Duration = Duration::from_secs(20);
const SILENCE: Duration = Duration::from_secs(90);

/// Two ends of a link within this process.
pub fn loopback() -> (ClientLink, HostLink) {
    let (up_tx, up_rx) = channel();
    let (down_tx, down_rx) = channel();
    (
        End {
            rx: up_rx,
            tx: down_tx,
            _types: std::marker::PhantomData,
        },
        End {
            rx: down_rx,
            tx: up_tx,
            _types: std::marker::PhantomData,
        },
    )
}

/// Ends a link over `stream`: a thread reads frames into the end's
/// channel and another writes the channel's messages out, so the end works
/// like a loopback one. Once the connection breaks (or the end is dropped)
/// the channels close and [`End::receive`] fails.
pub fn tcp<In, Out>(stream: TcpStream) -> std::io::Result<End<In, Out>> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(SILENCE))?;
    let mut reader = stream.try_clone()?;
    let mut writer = stream;
    let (in_tx, in_rx) = channel::<String>();
    let (out_tx, out_rx) = channel::<String>();
    std::thread::spawn(move || {
        let mut len = [0u8; 4];
        while reader.read_exact(&mut len).is_ok() {
            let n = u32::from_le_bytes(len) as usize;
            if n > MAX_FRAME {
                break;
            }
            // Keeping the link alive.
            if n == 0 {
                continue;
            }
            let mut buf = vec![0; n];
            if reader.read_exact(&mut buf).is_err() {
                break;
            }
            let Ok(text) = String::from_utf8(buf) else {
                break;
            };
            if in_tx.send(text).is_err() {
                break;
            }
        }
        let _ = reader.shutdown(std::net::Shutdown::Both);
    });
    std::thread::spawn(move || {
        loop {
            let text = match out_rx.recv_timeout(KEEPALIVE) {
                Ok(text) => text,
                Err(RecvTimeoutError::Timeout) => String::new(),
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let len = (text.len() as u32).to_le_bytes();
            if writer.write_all(&len).is_err() || writer.write_all(text.as_bytes()).is_err() {
                break;
            }
        }
        let _ = writer.shutdown(std::net::Shutdown::Both);
    });
    Ok(End {
        rx: in_rx,
        tx: out_tx,
        _types: std::marker::PhantomData,
    })
}

/// A host's listening socket.
pub struct Listener {
    sockets: Vec<TcpListener>,
    pub port: u16,
}

impl Listener {
    /// Listens on `port` of every IPv4 interface (0: any free port), and
    /// of every IPv6 one where the computer has IPv6.
    pub fn bind(port: u16) -> std::io::Result<Self> {
        let socket = TcpListener::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(true)?;
        let port = socket.local_addr()?.port();
        let mut sockets = vec![socket];
        if let Ok(v6) = TcpListener::bind(("::", port))
            && v6.set_nonblocking(true).is_ok()
        {
            sockets.push(v6);
        }
        Ok(Self { sockets, port })
    }

    /// A client that has just connected, if any.
    pub fn accept(&self) -> Option<ClientLink> {
        self.sockets.iter().find_map(|socket| {
            let (stream, _) = socket.accept().ok()?;
            stream.set_nonblocking(false).ok()?;
            tcp(stream).ok()
        })
    }
}

/// Connects to the host at `address` (`HOST` or `HOST:PORT`, the port
/// [`DEFAULT_PORT`] if not given) on another thread; the link, or why it
/// failed, arrives on the returned channel.
pub fn connect(address: &str) -> Receiver<Result<HostLink, String>> {
    let address = address.trim().to_owned();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(open(&address));
    });
    rx
}

fn open(address: &str) -> Result<HostLink, String> {
    let with_port = if address.parse::<std::net::SocketAddr>().is_ok()
        || address
            .rsplit_once(':')
            .is_some_and(|(h, p)| !h.is_empty() && !h.ends_with(':') && p.parse::<u16>().is_ok())
    {
        address.to_owned()
    } else {
        format!("{address}:{DEFAULT_PORT}")
    };
    let addrs: Vec<_> = with_port
        .to_socket_addrs()
        .map_err(|e| format!("{address}: {e}"))?
        .collect();
    let mut last = format!("{address}: no address");
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => return tcp(stream).map_err(|e| e.to_string()),
            Err(e) => last = format!("{address}: {e}"),
        }
    }
    Err(last)
}

/// This computer's address on its local network, as other computers there
/// reach it (none without a network).
pub fn lan_address() -> Option<std::net::IpAddr> {
    // Connecting a UDP socket sends nothing; it only picks the interface.
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

/// Digest of the game data that decides how a battle plays (the files of
/// `GLOBALS`, `SCENS` and `COLONIES`: names and sizes) and of the
/// protocol: a host and a client must agree on it.
pub fn data_digest(dirs: &[&Path]) -> String {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = format!(
                "{prefix}/{}",
                entry.file_name().to_string_lossy().to_ascii_uppercase()
            );
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                walk(&entry.path(), &name, out);
            } else {
                out.push((name, meta.len()));
            }
        }
    }
    let mut files = Vec::new();
    for (i, dir) in dirs.iter().enumerate() {
        walk(dir, &i.to_string(), &mut files);
    }
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    };
    eat(&PROTOCOL.to_le_bytes());
    for (name, len) in &files {
        eat(name.as_bytes());
        eat(&len.to_le_bytes());
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_carries_messages_both_ways() {
        let (host, client) = loopback();
        assert!(client.send(&ToHost::Request {
            command: Command::EndTurn
        }));
        assert_eq!(
            host.receive(),
            Ok(Some(ToHost::Request {
                command: Command::EndTurn
            }))
        );
        assert_eq!(host.receive(), Ok(None));
        let entry = Entry {
            player: 2,
            seed: Some(7),
            check: None,
            command: Command::Turn { unit: 3 },
        };
        assert!(host.send(&ToClient::Run {
            entry: entry.clone()
        }));
        assert_eq!(client.receive(), Ok(Some(ToClient::Run { entry })));
        drop(host);
        assert!(client.receive().is_err());
    }

    /// Waits for a message on `end` (the TCP threads deliver it soon).
    fn next<In: for<'de> Deserialize<'de>, Out: Serialize>(end: &End<In, Out>) -> In {
        for _ in 0..500 {
            if let Some(m) = end.receive().unwrap() {
                return m;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no message");
    }

    #[test]
    fn tcp_carries_messages_both_ways() {
        let listener = Listener::bind(0).unwrap();
        let joining = connect(&format!("127.0.0.1:{}", listener.port));
        let client = joining
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let host = (0..500)
            .find_map(|_| {
                let link = listener.accept();
                if link.is_none() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                link
            })
            .unwrap();
        let hello = ToHost::Hello {
            version: PROTOCOL,
            data: "x".into(),
            name: "Ann".into(),
        };
        assert!(client.send(&hello));
        assert_eq!(next(&host), hello);
        let chat = ToClient::Chat {
            slot: Some(1),
            text: "é".repeat(1000),
        };
        assert!(host.send(&chat));
        assert_eq!(next(&client), chat);
        drop(host);
        let gone = (0..500).any(|_| {
            std::thread::sleep(Duration::from_millis(10));
            client.receive().is_err()
        });
        assert!(gone);
    }

    #[test]
    fn empty_frames_keep_the_link_alive_unseen() {
        let listener = Listener::bind(0).unwrap();
        let mut raw = TcpStream::connect(("127.0.0.1", listener.port)).unwrap();
        let host = (0..500)
            .find_map(|_| {
                let link = listener.accept();
                if link.is_none() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                link
            })
            .unwrap();
        let hello = ToHost::Hello {
            version: PROTOCOL,
            data: "x".into(),
            name: "Ann".into(),
        };
        let text = serde_json::to_string(&hello).unwrap();
        raw.write_all(&0u32.to_le_bytes()).unwrap();
        raw.write_all(&(text.len() as u32).to_le_bytes()).unwrap();
        raw.write_all(text.as_bytes()).unwrap();
        assert_eq!(next(&host), hello);
    }

    #[test]
    fn digest_depends_on_the_files() {
        let dir = std::env::temp_dir().join(format!("openlibday-digest-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.txt"), "12").unwrap();
        let one = data_digest(&[&dir]);
        assert_eq!(one, data_digest(&[&dir]));
        std::fs::write(dir.join("sub/a.txt"), "123").unwrap();
        assert_ne!(one, data_digest(&[&dir]));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
