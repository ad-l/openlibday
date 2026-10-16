//! Network games (multiplayer MP2, `docs/multiplayer.md`): hosting and
//! joining over TCP, the WAITING AREA with its chat, and players leaving.
//!
//! The host listens for clients; a client's first message is
//! [`ToHost::Hello`], which the host answers with the client's slot or
//! with why it may not join. In the waiting area the host picks the
//! skirmish; once the battle is over everyone returns there. A client that
//! leaves mid-battle keeps its seat: the host deploys and ends its turns
//! for it, so the others play on. The host also chooses the game's
//! [`Settings`]; with a turn limit it ends a human's turn once their time
//! is up.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use libday_data::Dir;
use libday_view::scatter::Rand;
use libday_view::ui::Screen;
use serde::{Deserialize, Serialize};

use crate::Viewer;
use crate::command::{Command, Net};
use crate::discovery::{Announce, Responder};
use crate::link::{self, ClientLink, HostLink, Listener, Peer, ToClient, ToHost};
use crate::players::Players;
use crate::router::{PortMap, Wan};
use crate::skirmish::Phase;

/// Most humans in a game (the original's four player colours).
pub const MAX_HUMANS: usize = 4;
/// Longest player name and chat line.
const NAME_MAX: usize = 20;
const CHAT_MAX: usize = 120;
/// Chat lines kept.
const CHAT_LINES: usize = 100;

/// Longest turn limit, in minutes.
pub const TURN_MINUTES_MAX: u8 = 20;

/// What kind of game the waiting area starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Skirmish,
    Campaign,
}

/// What the host chooses in the waiting area; every client gets a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub mode: Mode,
    /// The campaign's difficulty, 0 to [`crate::front::DIFFICULTY_MAX`].
    pub difficulty: u8,
    /// How long a human's turn may last, in minutes; 0: no limit.
    pub turn_minutes: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Skirmish,
            difficulty: crate::front::DIFFICULTY_DEFAULT,
            turn_minutes: 0,
        }
    }
}

/// A setting the host changes with the arrows of the waiting area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Mode,
    Difficulty,
    TurnLimit,
}

/// A chat line: from a slot, or a notice from the game.
#[derive(Clone, Debug)]
pub struct Chat {
    pub slot: Option<i64>,
    pub text: String,
    /// When it arrived (the battle shows recent lines).
    pub at: Duration,
}

/// A network game's state beside the command layer's.
#[derive(Default)]
pub struct Online {
    /// On a host: where clients connect, and those not yet greeted.
    pub listener: Option<Listener>,
    pub joining: Vec<ClientLink>,
    /// On a client: the connection being opened.
    pub connecting: Option<Receiver<Result<HostLink, String>>>,
    /// This copy's slot (0 until a client is welcomed).
    pub slot: i64,
    /// The players' names by slot.
    pub names: BTreeMap<i64, String>,
    pub chat: Vec<Chat>,
    /// On a host: the slots of clients gone mid-battle.
    pub left: BTreeSet<i64>,
    /// A skirmish was started from the waiting area.
    pub started: bool,
    /// [`link::data_digest`] of this copy's game data.
    pub digest: String,
    pub settings: Settings,
    /// On a host: its address on the local network and port.
    pub lan: Option<String>,
    /// On a host: the port opened on the router for internet players, and
    /// the answers to LAN discovery (with the id telling this game apart).
    pub port_map: Option<PortMap>,
    pub responder: Option<Responder>,
    pub id: u64,
}

impl Online {
    pub fn is_host(&self) -> bool {
        self.listener.is_some()
    }
}

/// A number telling this hosted game from others found on the network.
fn game_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.finish()
}

/// `text` without control characters, at most `max` characters.
fn clean(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

impl Viewer {
    /// Whether this copy is in a network game (or hosting or joining one).
    pub(crate) fn online(&self) -> bool {
        self.net.online.is_some()
    }

    /// Whether other copies follow or feed this one (a network game, or
    /// the scripts' loopback links).
    pub(crate) fn networked(&self) -> bool {
        self.online() || self.net.host.is_some() || !self.net.peers.is_empty()
    }

    fn data_digest(&self) -> String {
        let dirs = [Dir::Globals, Dir::Scenarios, Dir::Colonies].map(|d| self.game.dir(d));
        link::data_digest(&dirs.iter().map(|p| p.as_path()).collect::<Vec<_>>())
    }

    /// The local player's name for others to see.
    fn my_name(&self, slot: i64) -> String {
        let name = clean(&self.front.name, NAME_MAX);
        if name.is_empty() {
            libday_view::minimap::player_name(slot)
        } else {
            name
        }
    }

    /// MULTI PLAYER: the SESSIONS dialog, out of any network game.
    pub(crate) fn open_sessions(&mut self) {
        self.open_main_menu();
        self.front.focus_address = false;
        self.screen = Screen::Sessions;
    }

    /// Drops any network game (its links close).
    pub(crate) fn leave_network(&mut self) {
        if self.online() {
            self.net = Net::default();
            self.seats = Players::default();
            self.front.chatting = false;
        }
    }

    /// HOST: listens on the chosen port and opens the waiting area.
    pub(crate) fn host_game(&mut self) {
        let port = self.front.port;
        let listener = match Listener::bind(port) {
            Ok(listener) => listener,
            Err(e) => {
                self.front.status = Some(format!("Cannot use port {port}: {e}"));
                return;
            }
        };
        self.leave_network();
        let port = listener.port;
        let lan = link::lan_address();
        let port_map = match lan {
            Some(std::net::IpAddr::V4(ip)) if self.front.router => Some(PortMap::open(port, ip)),
            _ => None,
        };
        let mut online = Online {
            listener: Some(listener),
            slot: 1,
            digest: self.data_digest(),
            lan: lan.map(|a| format!("{a}:{port}")),
            port_map,
            responder: self.front.discover.then(Responder::bind).flatten(),
            id: game_id(),
            ..Online::default()
        };
        online.names.insert(1, self.my_name(1));
        self.net = Net {
            seeds: Some(Rand::new(self.rng.rand() as u32)),
            online: Some(online),
            ..Net::default()
        };
        self.seats = Players::online(1, 1);
        self.front.status = None;
        self.screen = Screen::Lobby;
        self.chat_line(None, format!("Hosting on port {port}"));
    }

    /// JOIN: connects to the address typed in the SESSIONS dialog.
    pub(crate) fn join_game(&mut self) {
        let address = self.front.address.trim().to_owned();
        if address.is_empty() {
            self.front.status = Some("Type the host's address first".into());
            return;
        }
        self.leave_network();
        self.net.online = Some(Online {
            connecting: Some(link::connect(&address)),
            digest: self.data_digest(),
            ..Online::default()
        });
        self.front.status = Some(format!("Connecting to {address}..."));
    }

    /// Back to the waiting area once a battle is over (or the host leaves
    /// the selector).
    pub(crate) fn open_lobby(&mut self) {
        self.flow = crate::skirmish::Flow::new();
        self.front.chatting = false;
        self.net.requests.clear();
        let Some(online) = &mut self.net.online else {
            return;
        };
        online.started = false;
        if online.is_host() {
            for slot in std::mem::take(&mut online.left) {
                online.names.remove(&slot);
            }
            self.renumber();
        }
        self.screen = Screen::Lobby;
    }

    /// The host steps `setting` (in the waiting area) and tells everyone.
    pub(crate) fn change_setting(&mut self, setting: Setting, step: i8) {
        let Some(online) = &mut self.net.online else {
            return;
        };
        if !online.is_host() || online.started {
            return;
        }
        let s = &mut online.settings;
        match setting {
            Setting::Mode => {
                s.mode = match s.mode {
                    Mode::Skirmish => Mode::Campaign,
                    Mode::Campaign => Mode::Skirmish,
                };
            }
            Setting::Difficulty => {
                s.difficulty = s
                    .difficulty
                    .saturating_add_signed(step)
                    .min(crate::front::DIFFICULTY_MAX);
            }
            Setting::TurnLimit => {
                s.turn_minutes = s
                    .turn_minutes
                    .saturating_add_signed(step)
                    .min(TURN_MINUTES_MAX);
            }
        }
        self.send_settings();
    }

    /// The host sets the turn limit (`minutes`, 0 for none) at any time.
    pub(crate) fn set_turn_limit(&mut self, minutes: u8) {
        if let Some(online) = &mut self.net.online
            && online.is_host()
        {
            online.settings.turn_minutes = minutes.min(TURN_MINUTES_MAX);
            self.send_settings();
        }
    }

    fn send_settings(&self) {
        let Some(online) = &self.net.online else {
            return;
        };
        let message = ToClient::Settings {
            settings: online.settings,
        };
        for peer in &self.net.peers {
            peer.link.send(&message);
        }
    }

    /// Time left in the turn of the human playing now, in a network game
    /// with a turn limit.
    pub(crate) fn turn_left(&self) -> Option<Duration> {
        let minutes = self.net.online.as_ref()?.settings.turn_minutes;
        let player = self.map.battle.clock.player;
        if minutes == 0 || self.flow.phase != Phase::Player || !self.seats.human(player) {
            return None;
        }
        let limit = Duration::from_secs(u64::from(minutes) * 60);
        Some(limit.saturating_sub(self.clock.since(self.started)))
    }

    /// Copies `text` to the clipboard.
    pub(crate) fn copy_text(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.chat_line(None, format!("Copied {text}"));
        self.front.clipboard = Some(text);
    }

    /// START (host): the skirmish selector, whose choice starts everyone,
    /// or the co-op campaign.
    pub(crate) fn lobby_start(&mut self) {
        let settings = self.net.online.as_ref().map(|o| o.settings);
        if let Some(settings) = settings.filter(|s| s.mode == Mode::Campaign) {
            self.start_online_campaign(i64::from(settings.difficulty));
            return;
        }
        if let Err(e) = self.open_selector() {
            self.error = Some(e);
        }
    }

    /// BACK from the waiting area: leaves the game.
    pub(crate) fn lobby_back(&mut self) {
        self.open_sessions();
    }

    /// SEND: the typed chat line goes to everyone.
    pub(crate) fn send_chat(&mut self) {
        let text = clean(&std::mem::take(&mut self.front.chat), CHAT_MAX);
        if text.is_empty() {
            return;
        }
        if let Some(host) = &self.net.host {
            host.send(&ToHost::Chat { text });
        } else if let Some(slot) = self.net.online.as_ref().map(|o| o.slot) {
            self.post(Some(slot), text);
        }
    }

    /// The host shows a chat line and sends it to every client.
    fn post(&mut self, slot: Option<i64>, text: String) {
        let message = ToClient::Chat {
            slot,
            text: text.clone(),
        };
        for peer in &self.net.peers {
            peer.link.send(&message);
        }
        self.chat_line(slot, text);
    }

    fn chat_line(&mut self, slot: Option<i64>, text: String) {
        let at = self.clock.now();
        let Some(online) = &mut self.net.online else {
            return;
        };
        let theirs = slot.is_some_and(|s| s != online.slot);
        online.chat.push(Chat { slot, text, at });
        if online.chat.len() > CHAT_LINES {
            online.chat.remove(0);
        }
        if theirs {
            self.effect(crate::sound::Effect::Chat);
        }
    }

    /// The name of the player in `slot` in a network game.
    pub(crate) fn online_name(&self, slot: i64) -> Option<&str> {
        self.net
            .online
            .as_ref()?
            .names
            .get(&slot)
            .map(String::as_str)
    }

    /// Network work of a frame: a host greets new clients, a client
    /// finishes connecting.
    pub(crate) fn network_tick(&mut self) {
        self.search_tick();
        let Some(online) = &mut self.net.online else {
            return;
        };
        if let Some(rx) = &online.connecting {
            match rx.try_recv() {
                Ok(Ok(link)) => {
                    online.connecting = None;
                    link.send(&ToHost::Hello {
                        version: link::PROTOCOL,
                        data: online.digest.clone(),
                        name: self.my_name(0),
                    });
                    self.net.host = Some(link);
                    self.front.status = Some("Waiting for the host to answer...".into());
                }
                Ok(Err(e)) => {
                    self.leave_network();
                    self.front.status = Some(e);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.leave_network();
                    self.front.status = Some("Could not connect".into());
                }
            }
            return;
        }
        let Some(listener) = &online.listener else {
            return;
        };
        if let Some(responder) = &online.responder {
            responder.answer(&Announce {
                id: online.id,
                version: link::PROTOCOL,
                name: online.names.get(&1).cloned().unwrap_or_default(),
                port: listener.port,
                lan: online
                    .lan
                    .as_ref()
                    .and_then(|a| a.parse::<std::net::SocketAddr>().ok())
                    .map(|a| a.ip()),
                players: online.names.len(),
                max: MAX_HUMANS,
                started: online.started,
            });
        }
        let wan = online
            .port_map
            .as_mut()
            .and_then(|m| m.poll().then(|| m.wan.clone()));
        while let Some(link) = listener.accept() {
            online.joining.push(link);
        }
        let mut waiting = Vec::new();
        for link in std::mem::take(&mut online.joining) {
            match link.receive() {
                Ok(Some(ToHost::Hello {
                    version,
                    data,
                    name,
                })) => self.admit(link, version, &data, &name),
                Ok(None) => waiting.push(link),
                // Anything else first, or a broken link: not a client of ours.
                Ok(Some(_)) | Err(_) => {}
            }
        }
        if let Some(online) = &mut self.net.online {
            online.joining.extend(waiting);
        }
        if let Some(wan) = wan {
            self.wan_notice(&wan);
        }
    }

    /// Tells everyone how internet players can now join.
    fn wan_notice(&mut self, wan: &Wan) {
        let text = match wan {
            Wan::Searching => return,
            Wan::Open {
                address,
                method,
                private: false,
            } => format!(
                "Internet players can join at {address} ({} opened the port)",
                method.name()
            ),
            Wan::Open {
                address,
                private: true,
                ..
            } => format!(
                "The router opened the port, but its address {} is behind another router (carrier-grade NAT?): internet players may not reach it",
                address.ip()
            ),
            Wan::Failed(e) => {
                eprintln!("port mapping: {e}");
                format!(
                    "The router did not open the port (no UPnP or NAT-PMP): for internet players, forward TCP port {} to this computer",
                    self.net
                        .online
                        .as_ref()
                        .and_then(|o| o.listener.as_ref())
                        .map_or(0, |l| l.port)
                )
            }
        };
        self.post(None, text);
    }

    /// The host's answer to a client's hello.
    fn admit(&mut self, link: ClientLink, version: u32, data: &str, name: &str) {
        let Some(online) = &self.net.online else {
            return;
        };
        let refusal = if version != link::PROTOCOL {
            Some("The host runs another version of openlibday")
        } else if data != online.digest {
            Some("The host's game data differ from yours")
        } else if online.started {
            Some("The game has already started")
        } else if self.net.peers.len() + 1 >= MAX_HUMANS {
            Some("The game is full")
        } else {
            None
        };
        if let Some(reason) = refusal {
            link.send(&ToClient::Refuse {
                reason: reason.into(),
            });
            return;
        }
        let slot = (2..=MAX_HUMANS as i64)
            .find(|s| !self.net.peers.iter().any(|p| p.slot == *s))
            .unwrap_or(MAX_HUMANS as i64);
        let name = match clean(name, NAME_MAX) {
            n if n.is_empty() => libday_view::minimap::player_name(slot),
            n => n,
        };
        link.send(&ToClient::Welcome { slot });
        self.net.peers.push(Peer { slot, link });
        if let Some(online) = &mut self.net.online {
            online.names.insert(slot, name.clone());
        }
        self.renumber();
        self.post(None, format!("{name} joined"));
    }

    /// Slots 2 onwards in joining order with no gaps (telling clients
    /// whose slot changed), then everyone gets the names and the host
    /// seats the humans.
    fn renumber(&mut self) {
        let Some(online) = &mut self.net.online else {
            return;
        };
        self.net.peers.sort_by_key(|p| p.slot);
        let mut names = BTreeMap::new();
        if let Some(name) = online.names.get(&1) {
            names.insert(1, name.clone());
        }
        for (i, peer) in self.net.peers.iter_mut().enumerate() {
            let slot = i as i64 + 2;
            if let Some(name) = online.names.get(&peer.slot) {
                names.insert(slot, name.clone());
            }
            if peer.slot != slot {
                peer.slot = slot;
                peer.link.send(&ToClient::Welcome { slot });
            }
        }
        online.names = names;
        let roster = ToClient::Roster {
            players: online.names.clone().into_iter().collect(),
        };
        for peer in &self.net.peers {
            peer.link.send(&roster);
        }
        self.seats = Players::online(self.net.peers.len() + 1, 1);
        self.send_settings();
    }

    /// The host lost the client playing `slot`.
    pub(crate) fn peer_gone(&mut self, slot: i64) {
        self.net.peers.retain(|p| p.slot != slot);
        self.net.requests.retain(|(s, _)| *s != slot);
        let Some(online) = &mut self.net.online else {
            return;
        };
        let name = online
            .names
            .get(&slot)
            .cloned()
            .unwrap_or_else(|| libday_view::minimap::player_name(slot));
        if online.started {
            online.left.insert(slot);
            self.post(None, format!("{name} left; their turns end on their own"));
        } else {
            online.names.remove(&slot);
            self.renumber();
            self.post(None, format!("{name} left"));
        }
    }

    /// A client's message from the host.
    pub(crate) fn host_message(&mut self, message: ToClient) {
        match message {
            ToClient::Run { entry } => {
                self.net.inbox.get_or_insert_default().push_back(entry);
            }
            ToClient::Start { map, humans, slot } => {
                if let Some(online) = &mut self.net.online {
                    online.slot = slot;
                }
                self.join_skirmish(&map, humans, slot);
            }
            ToClient::Campaign {
                state,
                humans,
                slot,
                seed,
            } => self.follow_campaign(&state, humans, slot, seed),
            ToClient::Welcome { slot } => {
                let Some(online) = &mut self.net.online else {
                    return;
                };
                online.slot = slot;
                self.net.inbox.get_or_insert_default();
                self.seats = Players::online(slot as usize, slot);
                self.front.status = None;
                if self.screen == Screen::Sessions {
                    self.screen = Screen::Lobby;
                }
            }
            ToClient::Refuse { reason } => {
                self.leave_network();
                self.front.status = Some(reason);
            }
            ToClient::Roster { players } => {
                if let Some(online) = &mut self.net.online {
                    online.names = players.into_iter().collect();
                }
            }
            ToClient::Chat { slot, text } => self.chat_line(slot, text),
            ToClient::Settings { settings } => {
                if let Some(online) = &mut self.net.online {
                    online.settings = settings;
                }
            }
        }
    }

    /// The host passes a client's chat line on.
    pub(crate) fn relay_chat(&mut self, slot: i64, text: &str) {
        let text = clean(text, CHAT_MAX);
        if !text.is_empty() {
            self.post(Some(slot), text);
        }
    }

    /// A client lost its host.
    pub(crate) fn host_gone(&mut self, error: String) {
        if !self.online() {
            self.net.fault = Some(format!("host: {error}"));
            return;
        }
        self.net.host = None;
        let welcomed = self.net.online.as_ref().is_some_and(|o| o.slot > 0);
        self.open_sessions();
        self.front.status = Some(if welcomed {
            "The connection to the host was lost".into()
        } else {
            "The host closed the connection".into()
        });
    }

    /// What the host does for a player gone mid-battle, when it is up to
    /// them: deploy as they are, or end their turn; and for a player whose
    /// time is up: end their turn.
    pub(crate) fn stand_in(&mut self) -> Option<(i64, Command)> {
        let online = self.net.online.as_ref()?;
        let player = self.map.battle.clock.player;
        if online.is_host() && self.turn_left() == Some(Duration::ZERO) {
            if self.flow.reinforcing {
                return Some((player, Command::FinishReinforce));
            }
            if self.motions.is_empty() {
                let name = self.player_name(player);
                self.post(None, format!("{name}'s time is up"));
                return Some((player, Command::EndTurn));
            }
        }
        let online = self.net.online.as_ref()?;
        online.left.iter().find_map(|&slot| match self.flow.phase {
            Phase::Deploy if self.deploying(slot) => Some((slot, Command::Ready)),
            Phase::Player if self.turn_of(slot) && self.flow.reinforcing => {
                Some((slot, Command::FinishReinforce))
            }
            Phase::Player if self.turn_of(slot) && self.motions.is_empty() => {
                Some((slot, Command::EndTurn))
            }
            _ => None,
        })
    }
}
