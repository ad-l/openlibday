//! The command layer (multiplayer MP0, `docs/multiplayer.md`): every
//! action that changes the battle's rules state, a human's or the AI's, is a
//! [`Command`] that [`Viewer::submit`] runs. The interface checks what it
//! can and submits; the AI submits what it decides. Each run command is
//! logged with the player who gave it and, on a host handing out seeds, the
//! seed `rand()` restarts from, so that another copy of the battle given the
//! same entries in the same order (a client, or a replay) ends in the same
//! state ([`Viewer::state_hash`]).

use std::collections::VecDeque;

use libday_view::fire::{self, Fire, Target};
use libday_view::movement::{Step, executable_steps, move_budget};
use libday_view::scatter::Rand;
use serde::{Deserialize, Serialize};

use crate::Viewer;
use crate::base_editor::{Piece, Placed};
use crate::link::{HostLink, Peer, ToClient, ToHost};
use crate::skirmish::{AI, Phase};

/// What a unit fires at, as sent over the wire ([`Target`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aim {
    Unit(usize),
    Building(usize),
    Tile(i64, i64),
}

impl From<Target> for Aim {
    fn from(t: Target) -> Self {
        match t {
            Target::Unit(i) => Aim::Unit(i),
            Target::Building(i) => Aim::Building(i),
            Target::Tile(x, y) => Aim::Tile(x, y),
        }
    }
}

impl From<Aim> for Target {
    fn from(a: Aim) -> Self {
        match a {
            Aim::Unit(i) => Target::Unit(i),
            Aim::Building(i) => Target::Building(i),
            Aim::Tile(x, y) => Target::Tile(x, y),
        }
    }
}

/// One step of a walk `(x, y, facing, cost, stop)` ([`Step`]).
pub type Pace = (i32, i32, u8, u32, bool);

fn pace(s: &Step) -> Pace {
    (s.x, s.y, s.facing, s.cost, s.stop)
}

fn step(&(x, y, facing, cost, stop): &Pace) -> Step {
    Step {
        x,
        y,
        facing,
        cost,
        stop,
    }
}

/// An action changing the battle. Units are indices into the scenario's
/// unit list, the same on every copy of the battle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Command {
    /// The units (leader first) walk to `to`: a single unit along its
    /// path as far as its AP goes, a group onto `to` and around it.
    Move {
        units: Vec<usize>,
        to: (i64, i64),
    },
    /// The units (leader first) attack `target`, walking into range first
    /// if they must; `repeat` keeps firing while they can.
    Attack {
        units: Vec<usize>,
        target: Aim,
        repeat: bool,
    },
    /// Turns a unit one facing clockwise, for free.
    Turn {
        unit: usize,
    },
    Overwatch {
        unit: usize,
        on: bool,
    },
    AutoFire {
        on: bool,
    },
    /// Calls in automated unit `id` on `at` (a transport sets down on
    /// `dest`); a human pays for it.
    Support {
        id: i64,
        at: (i64, i64),
        dest: Option<(i64, i64)>,
    },
    /// The scenario's authored support `index` of this turn (the AI's).
    ScriptedSupport {
        index: usize,
    },
    /// Places a unit of type `kind` at level `tech` on `(x, y)` while
    /// deploying or reinforcing.
    Deploy {
        x: i64,
        y: i64,
        kind: i64,
        tech: i64,
    },
    /// Takes back the unit deployed this phase on `(x, y)`.
    Recall {
        x: i64,
        y: i64,
    },
    /// Buys the next technology level of setup cell `cell`.
    Upgrade {
        cell: usize,
    },
    /// The technology is bought: on to the briefing and deployment.
    Brief,
    /// The player has deployed; the battle starts once every human has.
    Ready,
    StartBattle,
    BeginReinforce,
    FinishReinforce,
    EndTurn,
    /// The AI's walk, already planned and cut to its AP.
    Walk {
        unit: usize,
        steps: Vec<Pace>,
        final_ap: i64,
    },
    /// The AI's walks playing together, each `(unit, steps, final_ap)`
    /// starting a little after the one before.
    Batch {
        walks: Vec<(usize, Vec<Pace>, i64)>,
    },
    /// The AI's fire from where the unit stands.
    Shoot {
        unit: usize,
        target: Aim,
        repeat: bool,
    },
    /// The AI's turn is over: the next human turn begins.
    EndAiTurn,
    /// Builds `piece` on the campaign base at `at` (a road or wall
    /// dragged from `start`).
    BaseBuild {
        piece: Piece,
        start: Option<(i64, i64)>,
        at: (i64, i64),
    },
    /// Moves a structure of the campaign base onto `to`.
    BaseMove {
        thing: Placed,
        to: (i64, i64),
    },
    /// Sells a structure of the campaign base.
    BaseRecycle {
        thing: Placed,
    },
    /// Buys the next technology level of unit type `kind` (ARMY screen).
    ArmyUpgrade {
        kind: i64,
    },
    /// The host invades campaign province `province`.
    Invade {
        province: i64,
    },
    /// The host closes the campaign battle report.
    Proceed,
}

/// A command as run: who gave it and the seed `rand()` restarted from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub player: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    /// On a host: the state hash the command was given in, which a client
    /// checks before running it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    #[serde(flatten)]
    pub command: Command,
}

/// The command log and how commands are fed.
#[derive(Default)]
pub struct Net {
    /// Every command run, in order.
    pub log: Vec<Entry>,
    /// On a host: where each command's seed comes from (none in single
    /// player, where `rand()` just runs on).
    pub seeds: Option<Rand>,
    /// On a client (or a replay): the commands still to run, each once
    /// nothing plays. The AI does not think here; its commands come in.
    pub inbox: Option<VecDeque<Entry>>,
    /// On a client: the first logged command run in a state other than
    /// the host's.
    pub desync: Option<usize>,
    /// On a host: the clients, and their requests still to run.
    pub peers: Vec<Peer>,
    pub requests: VecDeque<(i64, Command)>,
    /// On a client: the link to the host.
    pub host: Option<HostLink>,
    /// Why a link failed.
    pub fault: Option<String>,
    /// A network game's connections, players and chat.
    pub online: Option<crate::lobby::Online>,
}

impl Net {
    /// Whether this copy follows commands from elsewhere.
    pub fn follows(&self) -> bool {
        self.inbox.is_some()
    }
}

impl Viewer {
    /// Runs `command` for `player`, drawing it a seed on a host; returns
    /// whether it did anything (for a walk: whether it started).
    pub(crate) fn submit(&mut self, player: i64, command: Command) -> bool {
        let seed = self.net.seeds.as_mut().map(|r| r.rand() as u32);
        let check = seed.map(|_| format!("{:016x}", self.state_hash()));
        self.run(Entry {
            player,
            seed,
            check,
            command,
        })
    }

    /// Runs a logged command.
    pub(crate) fn run(&mut self, entry: Entry) -> bool {
        if self.net.follows()
            && self.net.desync.is_none()
            && let Some(check) = &entry.check
            && *check != format!("{:016x}", self.state_hash())
        {
            eprintln!("desync before command {}: {entry:?}", self.net.log.len());
            self.net.desync = Some(self.net.log.len());
        }
        if let Some(seed) = entry.seed {
            self.rng = Rand::new(seed);
        }
        self.net.log.push(entry.clone());
        if !self.net.follows() {
            let message = ToClient::Run {
                entry: entry.clone(),
            };
            for peer in &self.net.peers {
                peer.link.send(&message);
            }
        }
        self.apply(entry.player, entry.command)
    }

    /// Takes in what the links brought: a host queues its clients'
    /// requests, a client the host's commands (and starts the skirmish it
    /// is told to).
    pub(crate) fn receive(&mut self) {
        self.network_tick();
        let mut gone = Vec::new();
        let mut chat = Vec::new();
        for peer in &self.net.peers {
            loop {
                match peer.link.receive() {
                    Ok(Some(ToHost::Request { command })) => {
                        self.net.requests.push_back((peer.slot, command));
                    }
                    Ok(Some(ToHost::Chat { text })) => chat.push((peer.slot, text)),
                    Ok(Some(ToHost::Hello { .. })) => {}
                    Ok(None) => break,
                    Err(e) => {
                        gone.push((peer.slot, e));
                        break;
                    }
                }
            }
        }
        for (slot, text) in chat {
            self.relay_chat(slot, &text);
        }
        for (slot, e) in gone {
            if self.online() {
                self.peer_gone(slot);
            } else {
                self.net.fault = Some(format!("player {slot}: {e}"));
            }
        }
        while let Some(link) = &self.net.host {
            match link.receive() {
                Ok(Some(message)) => self.host_message(message),
                Ok(None) => break,
                Err(e) => {
                    self.host_gone(e);
                    break;
                }
            }
        }
    }

    /// Whether commands are waiting to run.
    pub(crate) fn pending(&self) -> bool {
        !self.net.requests.is_empty() || self.net.inbox.as_ref().is_some_and(|i| !i.is_empty())
    }

    /// Runs the next command received once nothing plays; returns whether
    /// one ran.
    pub(crate) fn pump(&mut self) -> bool {
        self.receive();
        if self.busy() {
            return false;
        }
        if let Some((player, command)) = self.net.requests.pop_front().or_else(|| self.stand_in()) {
            self.submit(player, command);
            return true;
        }
        let Some(entry) = self.net.inbox.as_mut().and_then(VecDeque::pop_front) else {
            return false;
        };
        self.run(entry);
        true
    }

    /// Whether `player` may give unit `unit` orders.
    fn commands(&self, player: i64, unit: usize) -> bool {
        let Some(u) = self.map.battle.scenario.units.get(unit) else {
            return false;
        };
        match self.flow.phase {
            Phase::Free => true,
            Phase::Player | Phase::Ai => {
                !self.flow.reinforcing
                    && u.owner == player
                    && self.map.battle.clock.player == player
            }
            _ => false,
        }
    }

    fn apply(&mut self, player: i64, command: Command) -> bool {
        match command {
            Command::Move { mut units, to } => {
                units.retain(|&u| self.commands(player, u));
                match units.len() {
                    0 => false,
                    1 => self.walk_to(units[0], to),
                    _ => {
                        self.move_units(units, to);
                        true
                    }
                }
            }
            Command::Attack {
                mut units,
                target,
                repeat,
            } => {
                units.retain(|&u| self.commands(player, u));
                match units.len() {
                    0 => false,
                    1 => {
                        self.attack_with(units[0], target.into(), repeat);
                        true
                    }
                    _ => {
                        self.fire_units(units, target.into(), repeat);
                        true
                    }
                }
            }
            Command::Turn { unit } => self.commands(player, unit) && self.apply_turn(unit),
            Command::Overwatch { unit, on } => {
                if !self.commands(player, unit) {
                    return false;
                }
                self.map.battle.scenario.units[unit].overwatch = on;
                self.refresh_preview();
                true
            }
            Command::AutoFire { on } => {
                self.map.battle.auto_fire = on;
                true
            }
            Command::Support { id, at, dest } => self.apply_support(player, id, at, dest),
            Command::ScriptedSupport { index } => {
                self.apply_scripted_support(index);
                true
            }
            Command::Deploy { x, y, kind, tech } => self.apply_deploy(player, x, y, kind, tech),
            Command::Recall { x, y } => self.apply_recall(player, x, y),
            Command::Upgrade { cell } => self.apply_upgrade(cell),
            Command::Brief => self.apply_brief(),
            Command::Ready => self.apply_ready(player),
            Command::StartBattle => {
                self.apply_start_battle();
                true
            }
            Command::BeginReinforce => self.apply_begin_reinforce(player),
            Command::FinishReinforce => self.apply_finish_reinforce(player),
            Command::EndTurn => self.apply_end_turn(player),
            Command::Walk {
                unit,
                steps,
                final_ap,
            } => {
                self.commands(player, unit)
                    && self.start_walk(unit, steps.iter().map(step).collect(), final_ap)
            }
            Command::Batch { walks } => {
                let mut any = false;
                for (nth, (unit, steps, final_ap)) in walks.into_iter().enumerate() {
                    if self.commands(player, unit) {
                        let steps = steps.iter().map(step).collect();
                        any |= self.start_batch_walk(unit, steps, final_ap, nth, false);
                    }
                }
                any
            }
            Command::Shoot {
                unit,
                target,
                repeat,
            } => {
                if !self.commands(player, unit) {
                    return false;
                }
                self.start_shot(unit, target.into(), repeat, false);
                self.shot.is_some()
            }
            Command::EndAiTurn => {
                self.apply_end_ai_turn();
                true
            }
            Command::BaseBuild { piece, start, at } => {
                self.apply_base_build(player, piece, start, at)
            }
            Command::BaseMove { thing, to } => self.apply_base_move(player, thing, to),
            Command::BaseRecycle { thing } => self.apply_base_recycle(player, thing),
            Command::ArmyUpgrade { kind } => self.apply_army_upgrade(player, kind),
            Command::Invade { province } => {
                player == self.seats.first_human() && self.apply_invade(province)
            }
            Command::Proceed => player == self.seats.first_human() && self.apply_proceed(),
        }
    }

    /// A single unit walks to `to` as far as its AP goes.
    fn walk_to(&mut self, unit: usize, to: (i64, i64)) -> bool {
        let Some(p) = self.plan(unit, to) else {
            return false;
        };
        let Some((ap, _)) = self.action_points(unit) else {
            return false;
        };
        let budget = move_budget(ap.max(0) as u32, self.reserve(unit));
        let (n, spent) = executable_steps(&p.steps, budget);
        let mut steps = p.steps;
        steps.truncate(n);
        self.start_walk(unit, steps, ap - i64::from(spent))
    }

    /// A single unit attacks `target`: fires if it can from where it
    /// stands, else walks into range and fires on arriving.
    fn attack_with(&mut self, unit: usize, target: Target, repeat: bool) {
        match fire::check(&self.map.battle.scenario, &self.defs, unit, target) {
            Some(Fire::Clear(_)) => self.start_shot(unit, target, repeat, false),
            Some(Fire::OutOfRange | Fire::TooClose | Fire::Blocked) => {
                let Some(p) = self.approach(unit, target) else {
                    return;
                };
                self.attack = Some((unit, target, repeat));
                if let Some(lock) = &mut self.lock
                    && lock.unit == unit
                {
                    lock.pending = true;
                }
                self.walk_to(unit, p.target);
                if self.motions.is_empty() && self.ends.is_empty() {
                    self.attack = None;
                    self.lock = None;
                }
            }
            _ => {}
        }
    }

    /// The AI's walk of `steps`.
    pub(crate) fn ai_walk(&mut self, unit: usize, steps: &[Step], final_ap: i64) -> bool {
        let steps = steps.iter().map(pace).collect();
        self.order_ai(Command::Walk {
            unit,
            steps,
            final_ap,
        })
    }

    /// The AI's walks of `walks` (`(unit, steps, final_ap)`), together.
    pub(crate) fn ai_batch(&mut self, walks: Vec<(usize, Vec<Step>, i64)>) -> bool {
        let walks = walks
            .into_iter()
            .map(|(u, s, ap)| (u, s.iter().map(pace).collect(), ap))
            .collect();
        self.order_ai(Command::Batch { walks })
    }

    /// The local human's order: run here, or on a client sent to the host
    /// (it runs when the host sends it back).
    pub(crate) fn order(&mut self, command: Command) -> bool {
        if let Some(link) = &self.net.host {
            return link.send(&ToHost::Request { command });
        }
        self.submit(self.seats.local, command)
    }

    /// The AI's order.
    pub(crate) fn order_ai(&mut self, command: Command) -> bool {
        self.submit(AI, command)
    }
}
