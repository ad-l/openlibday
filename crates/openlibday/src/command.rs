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
        self.apply(entry.player, entry.command)
    }

    /// Runs the next command received once nothing plays; returns whether
    /// one ran.
    pub(crate) fn pump(&mut self) -> bool {
        if self.busy() {
            return false;
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

    /// The local human's order.
    pub(crate) fn order(&mut self, command: Command) -> bool {
        self.submit(self.seats.local, command)
    }

    /// The AI's order.
    pub(crate) fn order_ai(&mut self, command: Command) -> bool {
        self.submit(AI, command)
    }
}
