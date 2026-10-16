//! Finding games on the local network (multiplayer MP4,
//! `docs/multiplayer.md`): the SESSIONS dialog broadcasts a question on
//! UDP [`DISCOVERY_PORT`] every second and hosts answer with an
//! [`Announce`], which the dialog lists.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The UDP port hosts listen on for the question.
pub const DISCOVERY_PORT: u16 = 41530;
/// The question.
const QUESTION: &[u8] = b"openlibday?";
/// How often the question goes out, and how long a silent game stays
/// listed.
const ASK_EVERY: Duration = Duration::from_secs(1);
const FORGET: Duration = Duration::from_secs(4);

/// A host's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announce {
    /// Tells one host from another whichever address it answered from.
    pub id: u64,
    pub version: u32,
    pub name: String,
    /// The TCP port to join on, and the host's main address on the local
    /// network (preferred when it answers on several networks).
    pub port: u16,
    pub lan: Option<IpAddr>,
    pub players: usize,
    pub max: usize,
    pub started: bool,
}

/// A host's socket answering the question.
pub struct Responder {
    socket: UdpSocket,
}

impl Responder {
    /// None if another program (or game hosted on this computer) has the
    /// port.
    pub fn bind() -> Option<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).ok()?;
        socket.set_nonblocking(true).ok()?;
        Some(Self { socket })
    }

    /// Answers every question received with `announce`.
    pub fn answer(&self, announce: &Announce) {
        let mut buf = [0u8; 64];
        while let Ok((n, from)) = self.socket.recv_from(&mut buf) {
            if buf.get(..n) == Some(QUESTION)
                && let Ok(text) = serde_json::to_vec(announce)
            {
                let _ = self.socket.send_to(&text, from);
            }
        }
    }
}

/// A game found.
#[derive(Clone, Debug)]
pub struct Found {
    pub address: SocketAddr,
    pub announce: Announce,
    seen: Instant,
}

/// The SESSIONS dialog's search.
pub struct Search {
    socket: UdpSocket,
    asked: Option<Instant>,
    pub games: Vec<Found>,
}

impl Search {
    pub fn new() -> Option<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        socket.set_broadcast(true).ok()?;
        socket.set_nonblocking(true).ok()?;
        Some(Self {
            socket,
            asked: None,
            games: Vec::new(),
        })
    }

    /// Asks again when due, takes the answers and forgets silent games.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if self
            .asked
            .is_none_or(|t| now.duration_since(t) >= ASK_EVERY)
        {
            self.asked = Some(now);
            for target in targets() {
                let _ = self.socket.send_to(QUESTION, target);
            }
        }
        let mut buf = [0u8; 1024];
        while let Ok((n, from)) = self.socket.recv_from(&mut buf) {
            let Ok(announce) = serde_json::from_slice::<Announce>(&buf[..n]) else {
                continue;
            };
            let address = SocketAddr::new(from.ip(), announce.port);
            match self.games.iter_mut().find(|g| g.announce.id == announce.id) {
                Some(game) => {
                    // A host answers on each network it is on (and on
                    // loopback when on this computer): show its main
                    // address, else any but loopback.
                    let rank = |ip: IpAddr| {
                        if Some(ip) == announce.lan {
                            2
                        } else {
                            u8::from(!ip.is_loopback())
                        }
                    };
                    if rank(address.ip()) > rank(game.address.ip()) {
                        game.address = address;
                    }
                    game.announce = announce;
                    game.seen = now;
                }
                None => self.games.push(Found {
                    address,
                    announce,
                    seen: now,
                }),
            }
        }
        self.games.retain(|g| now.duration_since(g.seen) < FORGET);
        self.games
            .sort_by(|a, b| a.announce.name.cmp(&b.announce.name));
    }
}

/// Where the question goes: every network's broadcast address, and this
/// computer.
fn targets() -> Vec<SocketAddr> {
    let mut ips = vec![Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST];
    for interface in netdev::get_interfaces() {
        if interface.is_up() && !interface.is_loopback() {
            ips.extend(interface.ipv4.iter().map(|net| net.broadcast()));
        }
    }
    ips.sort();
    ips.dedup();
    ips.into_iter()
        .map(|ip| SocketAddr::new(IpAddr::V4(ip), DISCOVERY_PORT))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_answer_the_question() {
        // Another copy of the game may hold the port: nothing to test then.
        let Some(responder) = Responder::bind() else {
            return;
        };
        let announce = Announce {
            id: 42,
            version: crate::link::PROTOCOL,
            name: "Ann".into(),
            port: 1234,
            lan: None,
            players: 1,
            max: 4,
            started: false,
        };
        let mut search = Search::new().unwrap();
        for _ in 0..300 {
            search.tick();
            responder.answer(&announce);
            if !search.games.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let game = &search.games[0];
        assert_eq!(game.announce, announce);
        assert_eq!(game.address.port(), 1234);
    }
}
