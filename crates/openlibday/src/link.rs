//! The connection between a host and its clients (multiplayer MP1,
//! `docs/multiplayer.md`). Messages travel as JSON text, so the loopback
//! link of one process exercises the encoding a network link will use.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use serde::{Deserialize, Serialize};

use crate::command::{Command, Entry};

/// A client's message to the host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum ToHost {
    /// An order for the host to validate, seed and run for the client's
    /// player.
    Request { command: Command },
}

/// The host's message to a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum ToClient {
    /// A skirmish begins on `map` with `humans` humans; the client plays
    /// `slot`.
    Start {
        map: String,
        humans: usize,
        slot: i64,
    },
    /// A command the host ran, for the client to run in turn.
    Run { entry: Entry },
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
}
