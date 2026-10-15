//! Mines met on the move, as the original's server move (`0x419e07`)
//! checks each tile a unit walks onto (`0x41a293`): a mine detector stops
//! when a mine hidden from its owner comes within its detection range and
//! reveals it (`0x41a57a`); otherwise an armed mine on the tile that
//! affects the unit stops it and explodes (`0x41aca5`), revealing the mines
//! around it to the unit's owner.

use libday_data::Definitions;
use libday_data::scenario::{Mine, Scenario};
use libday_sim::path::Movement;

/// `Mine Types.typeID` of the grav mine.
pub const GRAV_MINE: i64 = 1;

/// Whether a mine of kind `mine` goes off under a unit moving as
/// `movement`, `passing` over the tile or stopping on it (`0x436386`). The
/// mine types' `affectUnit_*` flags are not used.
pub fn affects(movement: Movement, mine: i64, passing: bool) -> bool {
    let grav = mine == GRAV_MINE;
    match movement {
        Movement::Ground | Movement::Trooper => true,
        Movement::Grav => grav,
        Movement::Air | Movement::Underwater => !passing,
        Movement::Teleport | Movement::GravFly => !passing && grav,
        Movement::Boat | Movement::NoneWater | Movement::NoneGround => false,
    }
}

/// Mines hidden from unit `unit`'s owner within its detection range of
/// `tile` (a square), armed or not.
pub fn detected(
    scenario: &Scenario,
    defs: &Definitions,
    unit: usize,
    tile: (i64, i64),
) -> Vec<usize> {
    let Some(u) = scenario.units.get(unit) else {
        return Vec::new();
    };
    let range = defs
        .unit_tech(u.kind, u.tech)
        .map_or(0, |k| k.mine_detection);
    if range <= 0 {
        return Vec::new();
    }
    scenario
        .mines
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.hidden_to(u.owner) && (m.x - tile.0).abs() <= range && (m.y - tile.1).abs() <= range
        })
        .map(|(i, _)| i)
        .collect()
}

/// The armed mine on `tile` that goes off under unit `unit`, whoever owns
/// it.
pub fn trigger(
    scenario: &Scenario,
    defs: &Definitions,
    unit: usize,
    tile: (i64, i64),
    passing: bool,
) -> Option<usize> {
    let u = scenario.units.get(unit)?;
    let movement = Movement::from_id(defs.unit(u.kind)?.movement);
    scenario
        .mines
        .iter()
        .position(|m| m.armed && (m.x, m.y) == tile && affects(movement, m.kind, passing))
}

/// Reveals the mines on the eight tiles around `at` to `player`.
pub fn reveal_around(mines: &mut [Mine], at: (i64, i64), player: i64) {
    for m in mines {
        let (dx, dy) = (m.x - at.0, m.y - at.1);
        if (dx, dy) != (0, 0) && dx.abs() <= 1 && dy.abs() <= 1 {
            m.reveal_to(player);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mine(x: i64, y: i64) -> Mine {
        Mine {
            counter: 0,
            kind: 0,
            owner: 0,
            x,
            y,
            armed: true,
            hidden_ai: false,
            hidden: [true; 4],
        }
    }

    #[test]
    fn grav_units_only_set_off_grav_mines() {
        assert!(affects(Movement::Ground, 0, true));
        assert!(affects(Movement::Trooper, 1, false));
        assert!(!affects(Movement::Grav, 0, true));
        assert!(affects(Movement::Grav, GRAV_MINE, true));
        assert!(!affects(Movement::Air, 0, true));
        assert!(affects(Movement::Air, 0, false));
        assert!(!affects(Movement::GravFly, 0, false));
        assert!(affects(Movement::GravFly, GRAV_MINE, false));
        assert!(!affects(Movement::Boat, GRAV_MINE, false));
    }

    #[test]
    fn hidden_and_revealed() {
        let mut m = mine(0, 0);
        assert!(m.hidden_to(1) && m.hidden_to(4) && !m.hidden_to(0));
        m.reveal_to(2);
        assert!(!m.hidden_to(1) && !m.hidden_to(3));
        let mut m = mine(0, 0);
        m.hidden_ai = true;
        m.reveal_to(0);
        assert!(!m.hidden_to(0) && m.hidden_to(1));
    }

    #[test]
    fn reveals_the_eight_neighbours() {
        let mut mines = vec![mine(5, 5), mine(6, 4), mine(4, 6), mine(7, 5)];
        reveal_around(&mut mines, (5, 5), 1);
        let hidden: Vec<bool> = mines.iter().map(|m| m.hidden_to(1)).collect();
        assert_eq!(hidden, [true, false, false, true]);
    }
}
