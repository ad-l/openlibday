//! Whether a shot lands on the aimed tile or scatters to a neighbour,
//! as the original decides it (`0x41a9fe`, called by the attack order
//! `0x41a7a3` for both normal and reaction fire).
//!
//! - Adjacent shots (Chebyshev distance 1 or less) always land.
//! - The shooter's hit chance is picked by distance band (`Ranges`):
//!   `HitShortRange`, `HitMediumRange` or `HitLongRange`.
//! - If a unit of the side whose turn it is stands on the aimed tile (so
//!   in practice the target of reaction fire), the chance is scaled by its
//!   `Jammer` percentage, truncated.
//! - `rand() % 100 <= chance` lands; otherwise `rand() % 8` picks one of
//!   the eight neighbours, kept only if it is on the map. Whatever stands
//!   there, friend or foe, takes the shot.

use libday_data::defs::{RangeBands, UnitType};

/// The MSVC runtime's `rand()`, which the original uses.
#[derive(Clone, Debug)]
pub struct Rand {
    state: u32,
}

impl Rand {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// Next value in `0..=0x7fff`.
    pub fn rand(&mut self) -> i64 {
        self.state = self.state.wrapping_mul(214_013).wrapping_add(2_531_011);
        i64::from((self.state >> 16) & 0x7fff)
    }
}

/// Neighbour offsets indexed by `rand() % 8` (table at `0x4aece0`).
pub const OFFSETS: [(i64, i64); 8] = [
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
];

/// Hit chance of `shooter` firing from `from` at `aim`, before the roll.
/// `evader` is the type of the unit on `aim` when it belongs to the side
/// whose turn it is. `None` when the shot cannot miss.
pub fn hit_chance(
    shooter: &UnitType,
    ranges: &RangeBands,
    from: (i64, i64),
    aim: (i64, i64),
    evader: Option<&UnitType>,
) -> Option<i64> {
    let d = (aim.0 - from.0).abs().max((aim.1 - from.1).abs());
    if d <= 1 {
        return None;
    }
    let hit = shooter.hit[ranges.band(d)];
    Some(match evader {
        Some(e) => (hit as f64 * e.jammer as f64 * 0.01) as i64,
        None => hit,
    })
}

/// The tile the shot actually lands on, on a `size` x `size` map.
pub fn land(
    rng: &mut Rand,
    shooter: &UnitType,
    ranges: &RangeBands,
    from: (i64, i64),
    aim: (i64, i64),
    evader: Option<&UnitType>,
    size: i64,
) -> (i64, i64) {
    let Some(hit) = hit_chance(shooter, ranges, from, aim, evader) else {
        return aim;
    };
    if rng.rand() % 100 <= hit {
        return aim;
    }
    let (dx, dy) = OFFSETS[(rng.rand() % 8) as usize];
    let (x, y) = (aim.0 + dx, aim.1 + dy);
    if (0..size).contains(&x) && (0..size).contains(&y) {
        (x, y)
    } else {
        aim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(hit: [i64; 3], jammer: i64) -> UnitType {
        UnitType {
            id: 0,
            tech: 0,
            name: String::new(),
            race: 0,
            facings: true,
            width: 1,
            height: 1,
            movement: 0,
            armor: 0,
            action_points: 0,
            shoot_action_points: 0,
            can_overwatch: false,
            move_pix_speed: 8,
            move_animation: false,
            move_start_animation: false,
            move_end_animation: false,
            image: String::new(),
            cost: 0,
            cost_to_next: 0,
            damage_base: 0,
            damage_random: 0,
            damage_type: 0,
            damage_area: 1,
            ruin: String::new(),
            range: 10,
            min_range: 0,
            has_weapon: true,
            anti_air: false,
            damage_aa: 0,
            mine_detection: 0,
            weapon_type: 0,
            jammer,
            hit,
            shot: Default::default(),
            ai_category: 0,
            is_convoy: false,
            is_ai_target: false,
        }
    }

    #[test]
    fn msvc_rand_sequence() {
        let mut r = Rand::new(1);
        let v: Vec<i64> = (0..5).map(|_| r.rand()).collect();
        assert_eq!(v, [41, 18467, 6334, 26500, 19169]);
    }

    #[test]
    fn bands_and_jammer() {
        let ranges = RangeBands::default();
        let k = kind([80, 66, 50], 100);
        let grav = kind([0, 0, 0], 10);
        assert_eq!(hit_chance(&k, &ranges, (0, 0), (1, 1), None), None);
        assert_eq!(hit_chance(&k, &ranges, (0, 0), (3, 2), None), Some(80));
        assert_eq!(hit_chance(&k, &ranges, (0, 0), (0, 4), None), Some(66));
        assert_eq!(hit_chance(&k, &ranges, (0, 0), (6, 6), None), Some(66));
        assert_eq!(hit_chance(&k, &ranges, (0, 0), (7, 0), None), Some(50));
        assert_eq!(
            hit_chance(&k, &ranges, (0, 0), (0, 4), Some(&grav)),
            Some(6)
        );
    }

    #[test]
    fn misses_land_on_a_neighbour_on_the_map() {
        let ranges = RangeBands::default();
        let never = kind([-1, -1, -1], 100);
        let mut rng = Rand::new(7);
        for _ in 0..100 {
            let at = land(&mut rng, &never, &ranges, (5, 5), (5, 9), None, 20);
            assert!(OFFSETS.contains(&(at.0 - 5, at.1 - 9)));
            let at = land(&mut rng, &never, &ranges, (5, 5), (0, 0), None, 20);
            assert!([(0, 0), (0, 1), (1, 0), (1, 1)].contains(&at));
        }
        let always = kind([99, 99, 99], 100);
        for _ in 0..100 {
            assert_eq!(
                land(&mut rng, &always, &ranges, (5, 5), (5, 9), None, 20),
                (5, 9)
            );
        }
    }
}
