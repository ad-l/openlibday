//! Overwatch and reaction fire, as the original's move order runs them
//! (`0x419e07`):
//!
//! - Before the unit walks, the path is checked tile by tile, from the tile
//!   it stands on (`0x419fdd`). It is *spotted* on a tile when a hostile
//!   unit there could fire at it on overwatch (`0x41a2fe`): one whose type
//!   `canOverwatch`, with at least a shot's AP left and a clear fire check
//!   on the tile. Stealth units (`Jammer` 0) are never spotted (`0x41a121`).
//!   The walk ends on the first tile it is spotted on, before mines are
//!   looked at there.
//! - Reaction fire then follows (`0x41a46b`): every hostile unit that can
//!   fire at it on overwatch shoots once, in unit order; rounds repeat while
//!   one did and the target stands. Each shot is resolved at once, with the
//!   target checked after every unit, so none is wasted. A unit's shots merge
//!   into one burst (`0x41b6b5`): raised once, fired back to back, lowered
//!   once; the shooters' bursts play together.
//!
//! No accuracy modifier is specific to overwatch: the target being of the
//! side whose turn it is, its `Jammer` scales the hit chance (`scatter`).

use std::collections::BTreeSet;

use libday_data::{Definitions, Scenario};

use crate::fire::{self, Fire, Target};

/// Whether unit `unit` is on the map and ready to fire on overwatch: its
/// type `canOverwatch` and it has a shot's AP left (`0x41a220`).
pub fn ready(scenario: &Scenario, defs: &Definitions, unit: usize) -> bool {
    let Some(u) = scenario.units.get(unit) else {
        return false;
    };
    let Some(kind) = defs.unit_tech(u.kind, u.tech) else {
        return false;
    };
    u.arrival == 0
        && kind.can_overwatch
        && kind.has_weapon
        && u.action_points.unwrap_or(kind.action_points) >= kind.shoot_action_points.max(0)
}

/// Whether unit `shooter` would fire on overwatch at unit `target`: it is
/// hostile (another owner), [`ready`], and has a clear shot.
pub fn reacts(scenario: &Scenario, defs: &Definitions, shooter: usize, target: usize) -> bool {
    let (Some(s), Some(t)) = (scenario.units.get(shooter), scenario.units.get(target)) else {
        return false;
    };
    shooter != target
        && s.owner != t.owner
        && t.arrival == 0
        && ready(scenario, defs, shooter)
        && matches!(
            fire::check(scenario, defs, shooter, Target::Unit(target)),
            Some(Fire::Clear(_))
        )
}

/// Whether moving unit `unit` is spotted where it stands: it is not
/// stealthy and a hostile unit [`reacts`] to it.
pub fn spotted(scenario: &Scenario, defs: &Definitions, unit: usize) -> bool {
    let Some(u) = scenario.units.get(unit) else {
        return false;
    };
    defs.unit_tech(u.kind, u.tech).is_some_and(|k| k.jammer > 0)
        && (0..scenario.units.len()).any(|s| reacts(scenario, defs, s, unit))
}

/// Tiles unit `unit` covers on overwatch: those it has a clear shot at
/// (in range and line of sight), when it is [`ready`].
pub fn zone(scenario: &Scenario, defs: &Definitions, unit: usize) -> BTreeSet<(i64, i64)> {
    let mut out = BTreeSet::new();
    if !ready(scenario, defs, unit) {
        return out;
    }
    let u = &scenario.units[unit];
    let Some(kind) = defs.unit_tech(u.kind, u.tech) else {
        return out;
    };
    let size = scenario.size as i64;
    let reach = kind.range.max(0);
    let (w, h) = (kind.width.max(1), kind.height.max(1));
    let xs = (u.x - reach).max(0)..=(u.x + w - 1 + reach).min(size - 1);
    for y in (u.y - reach).max(0)..=(u.y + h - 1 + reach).min(size - 1) {
        for x in xs.clone() {
            if matches!(
                fire::check(scenario, defs, unit, Target::Tile(x, y)),
                Some(Fire::Clear(_))
            ) {
                out.insert((x, y));
            }
        }
    }
    out
}
