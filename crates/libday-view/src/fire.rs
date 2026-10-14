//! Fire checks against a scenario: what blocks a line of sight.

use std::collections::BTreeMap;

use libday_data::{Definitions, Scenario};
pub use libday_sim::fire::Fire;
use libday_sim::fire::{self, Weapon};

/// Something a unit can be ordered to fire at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Index into `scenario.units`.
    Unit(usize),
    /// Index into `scenario.buildings`.
    Building(usize),
}

#[derive(Clone, Copy)]
enum Blocker {
    Solid,
    /// A building `(x, y, width, height)`.
    Building(i64, i64, i64, i64),
}

fn footprint(x: i64, y: i64, w: i64, h: i64) -> Vec<(i32, i32)> {
    (0..h)
        .flat_map(|dy| (0..w).map(move |dx| ((x + dx) as i32, (y + dy) as i32)))
        .collect()
}

/// Fire check of unit `attacker` (on the map) against `target`, with the
/// original's blockers between the two: terrain that cannot be fired
/// through, other units on the map, walls, ruins that cannot be fired
/// through, and buildings: a one-tile building always blocks; a larger one
/// does not when the aim is one of its corners other than the top-left.
/// `None` when either side has no definition.
pub fn check(
    scenario: &Scenario,
    defs: &Definitions,
    attacker: usize,
    target: Target,
) -> Option<Fire> {
    let unit = scenario.units.get(attacker)?;
    let kind = defs.unit_tech(unit.kind, unit.tech)?;
    let weapon = Weapon {
        has_weapon: kind.has_weapon,
        shot_cost: kind.shoot_action_points,
        range: kind.range,
        min_range: kind.min_range,
        indirect: kind.weapon_type == 1,
    };
    let from = (
        (unit.x + (kind.width - 1) / 2) as i32,
        (unit.y + (kind.height - 1) / 2) as i32,
    );
    let aims = match target {
        Target::Unit(i) => {
            let u = scenario.units.get(i)?;
            let k = defs.unit(u.kind)?;
            footprint(u.x, u.y, k.width, k.height)
        }
        Target::Building(i) => {
            let b = scenario.buildings.get(i)?;
            let k = defs.buildings.get(&b.kind)?;
            footprint(b.x, b.y, k.width, k.height)
        }
    };
    let mut blockers: BTreeMap<(i32, i32), Blocker> = BTreeMap::new();
    for (i, u) in scenario.units.iter().enumerate() {
        if i == attacker || u.arrival != 0 {
            continue;
        }
        let (w, h) = defs.unit(u.kind).map_or((1, 1), |k| (k.width, k.height));
        for tile in footprint(u.x, u.y, w, h) {
            blockers.insert(tile, Blocker::Solid);
        }
    }
    for w in &scenario.walls {
        blockers.insert((w.x as i32, w.y as i32), Blocker::Solid);
    }
    for r in &scenario.ruins {
        if defs.ruins.get(&r.id).is_some_and(|k| !k.can_fire_through) {
            blockers.insert((r.x as i32, r.y as i32), Blocker::Solid);
        }
    }
    for b in &scenario.buildings {
        let (w, h) = defs
            .buildings
            .get(&b.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        for tile in footprint(b.x, b.y, w, h) {
            blockers
                .entry(tile)
                .or_insert(Blocker::Building(b.x, b.y, w, h));
        }
    }
    let ap = unit.action_points.unwrap_or(kind.action_points);
    Some(fire::check(from, ap, &weapon, &aims, |tile, aim| {
        let terrain = scenario
            .cell(i64::from(tile.0), i64::from(tile.1))
            .and_then(|c| defs.terrain(&c.terrain));
        if terrain.is_some_and(|t| !t.can_fire_through) {
            return true;
        }
        match blockers.get(&tile) {
            None => false,
            Some(Blocker::Solid) => true,
            Some(&Blocker::Building(x, y, w, h)) => {
                let (right, bottom) = ((x + w - 1) as i32, (y + h - 1) as i32);
                let corners = [(right, y as i32), (x as i32, bottom), (right, bottom)];
                (w, h) == (1, 1) || !corners.contains(&aim)
            }
        }
    }))
}
