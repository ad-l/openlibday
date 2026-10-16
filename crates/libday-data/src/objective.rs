//! Scenario objectives: when a battle is won or lost.
//!
//! The original judges the objective (LIBDAY.EXE `0x43bd..`-`0x43ca..`, one
//! predicate per kind) after every action, after each AI step and when a
//! turn starts. Each predicate yields `done` and `won`; the common tail
//! `0x43c678` then applies forced outcomes (cheats) and the turn limit:
//! still undecided after the scenario's last turn is a defeat. Losing every
//! unit is not a defeat by itself.
//!
//! "Alive" means on the map: a unit that has arrived or a building.
//! Pending reinforcements are not alive. "On a tile" means the footprint
//! covers it.

use serde::{Deserialize, Serialize};

use crate::battle::Clock;
use crate::defs::Definitions;
use crate::scenario::{Objective, Scenario};

/// The AI's player slot.
pub const AI: i64 = 0;

/// Whether players `a` and `b` are enemies: the humans fight together
/// against the AI, so only the AI and a human are.
pub fn hostile(a: i64, b: i64) -> bool {
    (a == AI) != (b == AI)
}

/// `ObjectifId` values.
pub mod kind {
    pub const DESTROY: i64 = 0;
    pub const HOLD: i64 = 1;
    pub const ESCORT: i64 = 2;
    pub const PROTECT: i64 = 3;
    pub const INTERCEPT: i64 = 4;
    pub const KEEP: i64 = 5;
    pub const DESTROY_ALL_EXCEPT: i64 = 6;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Won,
    Lost,
}

type Rect = (i64, i64, i64, i64);

fn covers((x, y, w, h): Rect, tiles: &[(i64, i64)]) -> bool {
    tiles
        .iter()
        .any(|&(tx, ty)| (x..x + w).contains(&tx) && (y..y + h).contains(&ty))
}

fn unit_rect(defs: &Definitions, u: &crate::scenario::Unit) -> Rect {
    let (w, h) = defs.unit(u.kind).map_or((1, 1), |k| (k.width, k.height));
    (u.x, u.y, w, h)
}

/// Footprint of the unit or building `id` if it is on the map.
fn alive(s: &Scenario, defs: &Definitions, id: i64) -> Option<Rect> {
    if let Some(u) = s.units.iter().find(|u| u.id == id && u.arrival == 0) {
        return Some(unit_rect(defs, u));
    }
    let b = s.buildings.iter().find(|b| b.id == id)?;
    let (w, h) = defs
        .buildings
        .get(&b.kind)
        .map_or((1, 1), |k| (k.width, k.height));
    Some((b.x, b.y, w, h))
}

/// A unit of a player `side` accepts stands on one of `tiles`.
fn occupied(
    s: &Scenario,
    defs: &Definitions,
    tiles: &[(i64, i64)],
    side: impl Fn(i64) -> bool,
) -> bool {
    s.units
        .iter()
        .any(|u| u.arrival == 0 && side(u.owner) && covers(unit_rect(defs, u), tiles))
}

/// Judges `objective` on objective turn `turn` (the original's turn
/// counter: the battle turn, plus one during the AI's phase); `None` while
/// the battle goes on. `clock` keeps the hold counter (`objective_turn`)
/// and the turn it last counted (`last_turn`).
pub fn check(
    objective: &Objective,
    s: &Scenario,
    defs: &Definitions,
    clock: &mut Clock,
    turn: i64,
    forced: Option<Outcome>,
) -> Option<Outcome> {
    let turns = s.info.turns;
    let ids = &objective.ids;
    let tiles = &objective.tiles;
    let (mut done, mut won) = match objective.kind {
        // 0x43c11c: every listed ID destroyed, no road or mine left on the
        // listed tiles.
        kind::DESTROY => {
            let done = ids.iter().all(|&id| alive(s, defs, id).is_none())
                && !tiles.iter().any(|&t| {
                    s.roads.iter().any(|r| (r.x, r.y) == t)
                        || s.mines.iter().any(|m| (m.x, m.y) == t)
                });
            (done, done)
        }
        // 0x43c7b6: a non-AI unit on the tiles for more than `TurnNumber`
        // turns; a turn's absence is forgiven, two reset the count.
        kind::HOLD => {
            let present = occupied(s, defs, tiles, |o| o != AI);
            if present {
                if clock.last_turn != turn {
                    clock.objective_turn += 1;
                    clock.last_turn = turn;
                }
            } else if clock.last_turn + 2 <= turn {
                clock.last_turn = -1;
                clock.objective_turn = 0;
            }
            let count = clock.objective_turn;
            if count > objective.turn_number {
                (true, true)
            } else if turn > 0 && turn - count > turns - objective.turn_number + 1 {
                (true, false)
            } else {
                (false, false)
            }
        }
        // 0x43c70f: a listed unit reaches the tiles; lost once none is left.
        kind::ESCORT => {
            let mut found = false;
            let mut done = false;
            for &id in ids {
                if let Some(r) = alive(s, defs, id) {
                    found = true;
                    done |= covers(r, tiles);
                }
            }
            if found { (done, done) } else { (true, false) }
        }
        // 0x43c987: won after the last turn; lost if an AI unit stands on
        // the tiles on the last turn.
        kind::PROTECT => {
            if turn > turns {
                (true, true)
            } else if turn == turns && occupied(s, defs, tiles, |o| o == AI) {
                (true, false)
            } else {
                (false, false)
            }
        }
        // 0x43c87a: lost if a listed unit reaches the tiles, won once none
        // is left.
        kind::INTERCEPT => {
            let mut count = 0;
            let mut reached = false;
            for &id in ids {
                if let Some(r) = alive(s, defs, id) {
                    count += 1;
                    reached |= covers(r, tiles);
                }
            }
            if reached {
                (true, false)
            } else {
                (count == 0, count == 0)
            }
        }
        // 0x43c8fb: lost when a listed ID is destroyed, won after the last
        // turn.
        kind::KEEP => {
            let lost = ids.iter().any(|&id| alive(s, defs, id).is_none());
            if lost {
                (true, false)
            } else {
                (turn > turns, true)
            }
        }
        // 0x43c067: lost when a listed ID is destroyed, won once every
        // other AI unit is.
        kind::DESTROY_ALL_EXCEPT => {
            if ids.iter().any(|&id| alive(s, defs, id).is_none()) {
                (true, false)
            } else {
                let done = !s
                    .units
                    .iter()
                    .any(|u| u.arrival == 0 && u.owner == AI && !ids.contains(&u.id));
                (done, done)
            }
        }
        _ => (false, false),
    };
    // 0x43c678.
    if let Some(f) = forced {
        return Some(f);
    }
    // openlibday: a scenario without a turn limit (0) never times out.
    if !done && turn > 0 && turns > 0 && turn > turns {
        done = true;
        won = false;
    }
    done.then_some(if won { Outcome::Won } else { Outcome::Lost })
}

/// The hold count shown to the player on objective turn `turn`: it lapses
/// once a turn passed without a unit on the tiles.
pub fn held(clock: &Clock, turn: i64) -> i64 {
    if clock.last_turn == -1 || turn <= clock.last_turn + 1 {
        clock.objective_turn
    } else {
        0
    }
}

/// The objective as one line of text.
pub fn describe(objective: &Objective) -> &'static str {
    match objective.kind {
        kind::DESTROY => "destroy the targets",
        kind::HOLD => "hold the area",
        kind::ESCORT => "escort the units to the area",
        kind::PROTECT => "keep the enemy out of the area",
        kind::INTERCEPT => "stop the units reaching the area",
        kind::KEEP => "keep the units alive",
        kind::DESTROY_ALL_EXCEPT => "destroy every enemy unit but the listed ones",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{BuildingType, UnitType};
    use crate::scenario::{Building, Mine, Order, ScenarioInfo, Surface, Unit};

    #[test]
    fn humans_are_allies() {
        assert!(hostile(AI, 1) && hostile(3, AI));
        assert!(!hostile(1, 2) && !hostile(4, 1) && !hostile(2, 2) && !hostile(AI, AI));
    }

    fn defs() -> Definitions {
        let mut d = Definitions::default();
        let k = UnitType {
            id: 1,
            width: 1,
            height: 1,
            ..UnitType::default()
        };
        d.units.insert((1, 0), k);
        let b = BuildingType {
            id: 7,
            width: 2,
            height: 2,
            ..BuildingType::default()
        };
        d.buildings.insert(7, b);
        d
    }

    fn unit(id: i64, owner: i64, x: i64, y: i64) -> Unit {
        Unit {
            counter: id,
            id,
            kind: 1,
            tech: 0,
            owner,
            facing: 0,
            x,
            y,
            arrival: 0,
            reinforcement: false,
            armor: Some(1),
            action_points: Some(1),
            neg_ap: 0,
            order: Order::default(),
            objective_order: false,
            overwatch: true,
        }
    }

    fn scenario(turns: i64, units: Vec<Unit>) -> Scenario {
        Scenario {
            size: 16,
            cells: Vec::new(),
            buildings: Vec::new(),
            units,
            roads: Vec::new(),
            walls: Vec::new(),
            ruins: Vec::new(),
            mines: Vec::new(),
            deployments: Vec::new(),
            order_targets: Vec::new(),
            objective: None,
            info: ScenarioInfo {
                turns,
                ..ScenarioInfo::default()
            },
        }
    }

    fn objective(kind: i64, turn_number: i64, ids: &[i64], tiles: &[(i64, i64)]) -> Objective {
        Objective {
            kind,
            turn_number,
            ids: ids.to_vec(),
            tiles: tiles.to_vec(),
        }
    }

    fn judge(o: &Objective, s: &Scenario, turn: i64) -> Option<Outcome> {
        check(o, s, &defs(), &mut Clock::default(), turn, None)
    }

    #[test]
    fn destroy_needs_ids_gone_and_tiles_cleared() {
        let o = objective(kind::DESTROY, 0, &[5, 1000001], &[(3, 3)]);
        let mut s = scenario(10, vec![unit(5, AI, 0, 0)]);
        s.buildings.push(Building {
            counter: 1,
            id: 1000001,
            kind: 7,
            owner: AI,
            armor: 10,
            x: 8,
            y: 8,
            ai_target: false,
        });
        s.roads.push(Surface {
            counter: 1,
            kind: 0,
            owner: 0,
            x: 3,
            y: 3,
            armor: 0,
            broken: false,
        });
        assert_eq!(judge(&o, &s, 1), None);
        s.units.clear();
        s.buildings.clear();
        assert_eq!(judge(&o, &s, 1), None, "road left");
        s.roads.clear();
        s.mines.push(Mine {
            counter: 1,
            kind: 0,
            owner: 0,
            x: 3,
            y: 3,
            armed: true,
            hidden_ai: false,
            hidden: [false; 4],
        });
        assert_eq!(judge(&o, &s, 1), None, "mine left");
        s.mines.clear();
        assert_eq!(judge(&o, &s, 1), Some(Outcome::Won));
    }

    #[test]
    fn running_out_of_turns_loses() {
        let o = objective(kind::DESTROY, 0, &[5], &[]);
        let s = scenario(10, vec![unit(5, AI, 0, 0)]);
        assert_eq!(judge(&o, &s, 10), None);
        assert_eq!(judge(&o, &s, 11), Some(Outcome::Lost));
        // Without a limit the battle goes on.
        assert_eq!(judge(&o, &scenario(0, vec![unit(5, AI, 0, 0)]), 99), None);
    }

    #[test]
    fn forced_outcomes_win_over_the_objective() {
        let o = objective(kind::DESTROY, 0, &[5], &[]);
        let s = scenario(10, vec![unit(5, AI, 0, 0)]);
        let mut c = Clock::default();
        let f = Some(Outcome::Won);
        assert_eq!(check(&o, &s, &defs(), &mut c, 1, f), Some(Outcome::Won));
    }

    #[test]
    fn hold_counts_turns_and_forgives_one_absence() {
        let o = objective(kind::HOLD, 2, &[], &[(4, 4)]);
        let here = scenario(10, vec![unit(1, 1, 4, 4)]);
        let away = scenario(10, vec![unit(1, 1, 0, 0)]);
        let d = defs();
        let mut c = Clock::default();
        assert_eq!(check(&o, &here, &d, &mut c, 1, None), None);
        // Checked again the same turn: counted once.
        assert_eq!(check(&o, &here, &d, &mut c, 1, None), None);
        assert_eq!((c.objective_turn, c.last_turn), (1, 1));
        // Away for turn 2: kept; back on turn 3 it counts on.
        assert_eq!(check(&o, &away, &d, &mut c, 2, None), None);
        assert_eq!(held(&c, 2), 1);
        assert_eq!(check(&o, &here, &d, &mut c, 3, None), None);
        assert_eq!(c.objective_turn, 2);
        assert_eq!(check(&o, &here, &d, &mut c, 4, None), Some(Outcome::Won));
        // Two turns away reset it.
        let mut c = Clock::default();
        check(&o, &here, &d, &mut c, 1, None);
        check(&o, &away, &d, &mut c, 2, None);
        assert_eq!(held(&c, 3), 0);
        check(&o, &away, &d, &mut c, 3, None);
        assert_eq!((c.objective_turn, c.last_turn), (0, -1));
    }

    #[test]
    fn hold_is_lost_once_too_late_to_hold() {
        // 10 turns, hold 2: from turn 10 - 2 + 1 + 1 = 10 without holding
        // there is no time left.
        let o = objective(kind::HOLD, 2, &[], &[(4, 4)]);
        let away = scenario(10, vec![unit(1, 1, 0, 0)]);
        assert_eq!(judge(&o, &away, 9), None);
        assert_eq!(judge(&o, &away, 10), Some(Outcome::Lost));
        // AI units do not hold.
        let ai = scenario(10, vec![unit(1, AI, 4, 4)]);
        let d = defs();
        let mut c = Clock::default();
        check(&o, &ai, &d, &mut c, 1, None);
        assert_eq!(c.objective_turn, 0);
    }

    #[test]
    fn escort_wins_on_arrival_and_loses_without_escorts() {
        let o = objective(kind::ESCORT, 0, &[3, 4], &[(9, 9)]);
        assert_eq!(judge(&o, &scenario(10, vec![unit(3, 1, 0, 0)]), 1), None);
        assert_eq!(
            judge(
                &o,
                &scenario(10, vec![unit(3, 1, 0, 0), unit(4, 1, 9, 9)]),
                1
            ),
            Some(Outcome::Won)
        );
        assert_eq!(judge(&o, &scenario(10, vec![]), 1), Some(Outcome::Lost));
        // Pending reinforcements do not count.
        let mut pending = unit(3, 1, 9, 9);
        pending.arrival = 4;
        assert_eq!(
            judge(&o, &scenario(10, vec![pending]), 1),
            Some(Outcome::Lost)
        );
    }

    #[test]
    fn protect_is_judged_on_the_last_turn() {
        let o = objective(kind::PROTECT, 0, &[], &[(2, 2)]);
        let ai_in = scenario(10, vec![unit(1, AI, 2, 2)]);
        assert_eq!(judge(&o, &ai_in, 9), None);
        assert_eq!(judge(&o, &ai_in, 10), Some(Outcome::Lost));
        let mine_in = scenario(10, vec![unit(1, 1, 2, 2)]);
        assert_eq!(judge(&o, &mine_in, 10), None);
        assert_eq!(judge(&o, &mine_in, 11), Some(Outcome::Won));
    }

    #[test]
    fn intercept_loses_on_arrival_and_wins_when_all_destroyed() {
        let o = objective(kind::INTERCEPT, 0, &[3], &[(9, 9)]);
        assert_eq!(judge(&o, &scenario(10, vec![unit(3, AI, 0, 0)]), 1), None);
        assert_eq!(
            judge(&o, &scenario(10, vec![unit(3, AI, 9, 9)]), 1),
            Some(Outcome::Lost)
        );
        assert_eq!(judge(&o, &scenario(10, vec![]), 1), Some(Outcome::Won));
    }

    #[test]
    fn keep_wins_after_the_last_turn() {
        let o = objective(kind::KEEP, 0, &[3], &[]);
        let s = scenario(10, vec![unit(3, 1, 0, 0)]);
        assert_eq!(judge(&o, &s, 10), None);
        assert_eq!(judge(&o, &s, 11), Some(Outcome::Won));
        assert_eq!(judge(&o, &scenario(10, vec![]), 1), Some(Outcome::Lost));
    }

    #[test]
    fn destroy_all_except_spares_the_listed_units() {
        let o = objective(kind::DESTROY_ALL_EXCEPT, 0, &[3], &[]);
        let both = scenario(10, vec![unit(3, AI, 0, 0), unit(4, AI, 1, 1)]);
        assert_eq!(judge(&o, &both, 1), None);
        let spared = scenario(10, vec![unit(3, AI, 0, 0), unit(5, 1, 1, 1)]);
        assert_eq!(judge(&o, &spared, 1), Some(Outcome::Won));
        assert_eq!(
            judge(&o, &scenario(10, vec![unit(4, AI, 1, 1)]), 1),
            Some(Outcome::Lost)
        );
    }
}
