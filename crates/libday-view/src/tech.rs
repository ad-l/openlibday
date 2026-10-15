//! Technology capture (`0x41ae03`, the server's `CTMMsgTech` source).
//!
//! Buildings with a `TechReward` (tech centres and the ancient ruins) hold
//! technology. A building is captured while a unit of a human player
//! (owner other than the AI, `+0x1c != 0`) stands on one of the tiles
//! ringing its footprint (the eight-neighbourhood, map tiles only), and
//! released as soon as none does. The original re-judges this after every
//! move, attack, new map object and phase start, and once more when the
//! battle ends: the battle report's "New technologies found" is the sum of
//! the rewards captured then (`0x41b07a`).

use std::collections::BTreeSet;

use libday_data::objective::AI;
use libday_data::scenario::Scenario;
use libday_data::{Definitions, scenario::Building};

/// The reward of building `b`, if it holds technology.
pub fn reward(defs: &Definitions, b: &Building) -> Option<i64> {
    defs.buildings
        .get(&b.kind)
        .map(|k| k.tech_reward)
        .filter(|&r| r > 0)
}

/// Footprint `(x, y, w, h)` of building `b`.
fn rect(defs: &Definitions, b: &Building) -> (i64, i64, i64, i64) {
    let (w, h) = defs
        .buildings
        .get(&b.kind)
        .map_or((1, 1), |k| (k.width, k.height));
    (b.x, b.y, w, h)
}

/// The on-map tiles ringing building `b` (`0x48af9f`).
pub fn ring(scenario: &Scenario, defs: &Definitions, b: &Building) -> Vec<(i64, i64)> {
    let (x, y, w, h) = rect(defs, b);
    let size = scenario.size as i64;
    let mut tiles = Vec::new();
    for tx in x - 1..=x + w {
        for ty in y - 1..=y + h {
            let inside = (x..x + w).contains(&tx) && (y..y + h).contains(&ty);
            if !inside && (0..size).contains(&tx) && (0..size).contains(&ty) {
                tiles.push((tx, ty));
            }
        }
    }
    tiles
}

/// Ids of the technology buildings a human unit stands next to.
pub fn captured(scenario: &Scenario, defs: &Definitions) -> BTreeSet<i64> {
    let humans: Vec<(i64, i64, i64, i64)> = scenario
        .units
        .iter()
        .filter(|u| u.arrival == 0 && u.owner != AI)
        .map(|u| {
            let (w, h) = defs.unit(u.kind).map_or((1, 1), |k| (k.width, k.height));
            (u.x, u.y, w, h)
        })
        .collect();
    scenario
        .buildings
        .iter()
        .filter(|b| reward(defs, b).is_some())
        .filter(|b| {
            ring(scenario, defs, b).into_iter().any(|(tx, ty)| {
                humans
                    .iter()
                    .any(|&(x, y, w, h)| (x..x + w).contains(&tx) && (y..y + h).contains(&ty))
            })
        })
        .map(|b| b.id)
        .collect()
}

/// Technology the captured buildings `ids` are worth.
pub fn total(scenario: &Scenario, defs: &Definitions, ids: &BTreeSet<i64>) -> i64 {
    scenario
        .buildings
        .iter()
        .filter(|b| ids.contains(&b.id))
        .filter_map(|b| reward(defs, b))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use libday_data::defs::{BuildingType, UnitType};
    use libday_data::scenario::{Order, ScenarioInfo, Unit};

    fn defs() -> Definitions {
        let mut d = Definitions::default();
        d.units.insert(
            (1, 0),
            UnitType {
                id: 1,
                width: 1,
                height: 1,
                ..UnitType::default()
            },
        );
        for (id, reward) in [(235, 1000), (130, 0)] {
            d.buildings.insert(
                id,
                BuildingType {
                    id,
                    width: 3,
                    height: 3,
                    tech_reward: reward,
                    ..BuildingType::default()
                },
            );
        }
        d
    }

    fn building(id: i64, kind: i64, x: i64, y: i64) -> Building {
        Building {
            counter: id,
            id,
            kind,
            owner: 0,
            armor: 100,
            x,
            y,
            ai_target: false,
        }
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

    fn scenario(units: Vec<Unit>) -> Scenario {
        Scenario {
            size: 16,
            cells: Vec::new(),
            buildings: vec![building(1, 235, 5, 5), building(2, 130, 10, 10)],
            units,
            roads: Vec::new(),
            walls: Vec::new(),
            ruins: Vec::new(),
            mines: Vec::new(),
            deployments: Vec::new(),
            order_targets: Vec::new(),
            objective: None,
            info: ScenarioInfo::default(),
        }
    }

    #[test]
    fn ring_surrounds_the_footprint() {
        let s = scenario(Vec::new());
        let d = defs();
        assert_eq!(ring(&s, &d, &s.buildings[0]).len(), 16);
        let corner = building(3, 235, 0, 0);
        assert_eq!(ring(&s, &d, &corner).len(), 7);
    }

    #[test]
    fn human_units_next_to_tech_capture_it() {
        let d = defs();
        for (x, y) in [(4, 4), (8, 6), (6, 8), (4, 7)] {
            let s = scenario(vec![unit(9, 1, x, y)]);
            assert_eq!(captured(&s, &d), BTreeSet::from([1]), "({x}, {y})");
            assert_eq!(total(&s, &d, &captured(&s, &d)), 1000);
        }
        for (x, y) in [(3, 5), (9, 5), (5, 9)] {
            let s = scenario(vec![unit(9, 1, x, y)]);
            assert!(captured(&s, &d).is_empty(), "({x}, {y})");
        }
        // The AI never captures; buildings without a reward never count.
        assert!(captured(&scenario(vec![unit(9, AI, 4, 4)]), &d).is_empty());
        assert!(captured(&scenario(vec![unit(9, 1, 9, 9)]), &d).is_empty());
        let mut away = unit(9, 1, 4, 4);
        away.arrival = 2;
        assert!(captured(&scenario(vec![away]), &d).is_empty());
    }
}
