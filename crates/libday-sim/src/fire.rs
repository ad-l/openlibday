//! Whether a unit can fire at a target: action points, range and line of
//! sight, as the original's fire check (`0x435ceb`) decides it.

/// Outcome of a fire check; the discriminants are the original error codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fire {
    /// The target can be hit, aiming at this tile of its footprint.
    Clear((i32, i32)),
    /// Off the map, or not a valid target (the attacker itself).
    Invalid,
    /// The unit has no ground weapon.
    NoWeapon,
    /// Fewer action points left than a shot costs.
    NoActionPoints,
    /// Every footprint tile beyond the weapon range.
    OutOfRange,
    /// Within the weapon's minimum range.
    TooClose,
    /// In range, but the line of sight is blocked.
    Blocked,
}

impl Fire {
    pub fn code(self) -> u8 {
        match self {
            Fire::Clear(_) => 0,
            Fire::Invalid => 1,
            Fire::NoWeapon => 2,
            Fire::NoActionPoints => 3,
            Fire::OutOfRange => 4,
            Fire::TooClose => 5,
            Fire::Blocked => 6,
        }
    }
}

/// The attacker's weapon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Weapon {
    pub has_weapon: bool,
    /// AP of one shot.
    pub shot_cost: i64,
    /// Targets must be at most this many tiles away (Chebyshev distance).
    pub range: i64,
    /// ... and farther than this.
    pub min_range: i64,
    /// Indirect fire ignores the line of sight.
    pub indirect: bool,
}

/// Tiles of the line from `a` to `b`, both included, in that order: the
/// original's Bresenham walk, always traced from the endpoint with the
/// smaller `y` (so `a`→`b` and `b`→`a` cover the same tiles).
pub fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let reversed = a.1 > b.1;
    let (from, to) = if reversed { (b, a) } else { (a, b) };
    let (dx, dy) = ((to.0 - from.0).abs(), to.1 - from.1);
    let step = if to.0 >= from.0 { 1 } else { -1 };
    let (mut x, mut y) = from;
    let mut points = vec![from];
    if dx > dy {
        let mut d = 2 * dy - dx;
        for _ in 0..dx {
            if d >= 0 {
                y += 1;
                d += 2 * dy - 2 * dx;
            } else {
                d += 2 * dy;
            }
            x += step;
            points.push((x, y));
        }
    } else {
        let mut d = 2 * dx - dy;
        for _ in 0..dy {
            if d >= 0 {
                x += step;
                d += 2 * dx - 2 * dy;
            } else {
                d += 2 * dx;
            }
            y += 1;
            points.push((x, y));
        }
    }
    if reversed {
        points.reverse();
    }
    points
}

/// Chebyshev distance.
pub fn distance(a: (i32, i32), b: (i32, i32)) -> i64 {
    i64::from((a.0 - b.0).abs().max((a.1 - b.1).abs()))
}

/// Fire check of an attacker at `from` with `action_points` left against a
/// target covering `footprint`. `blocks(tile, aim)` tells whether `tile`
/// (strictly between the two ends) stops a shot aimed at tile `aim`.
///
/// Footprint tiles are tried in order; the first in range with a clear line
/// is the aim. Otherwise the result is `OutOfRange` if any tile is beyond
/// the range, else `TooClose` if any is within the minimum range, else
/// `Blocked` (the original's precedence, also for multi-tile targets).
pub fn check(
    from: (i32, i32),
    action_points: i64,
    weapon: &Weapon,
    footprint: &[(i32, i32)],
    blocks: impl Fn((i32, i32), (i32, i32)) -> bool,
) -> Fire {
    if footprint.is_empty() || footprint.contains(&from) {
        return Fire::Invalid;
    }
    if !weapon.has_weapon {
        return Fire::NoWeapon;
    }
    if action_points < weapon.shot_cost {
        return Fire::NoActionPoints;
    }
    let (mut all_in_range, mut all_beyond_min) = (true, true);
    for &aim in footprint {
        let d = distance(from, aim);
        let (in_range, beyond_min) = (d <= weapon.range, d > weapon.min_range);
        all_in_range &= in_range;
        all_beyond_min &= beyond_min;
        if in_range && beyond_min && (weapon.indirect || sight(from, aim, &blocks)) {
            return Fire::Clear(aim);
        }
    }
    if !all_in_range {
        Fire::OutOfRange
    } else if !all_beyond_min {
        Fire::TooClose
    } else {
        Fire::Blocked
    }
}

/// Whether nothing between `from` and `aim` blocks the shot.
fn sight(
    from: (i32, i32),
    aim: (i32, i32),
    blocks: &impl Fn((i32, i32), (i32, i32)) -> bool,
) -> bool {
    let points = line(from, aim);
    let inner = &points[1..points.len().saturating_sub(1).max(1)];
    !inner.iter().any(|&p| blocks(p, aim))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIFLE: Weapon = Weapon {
        has_weapon: true,
        shot_cost: 40,
        range: 5,
        min_range: 0,
        indirect: false,
    };

    #[test]
    fn lines() {
        assert_eq!(line((0, 0), (3, 0)), [(0, 0), (1, 0), (2, 0), (3, 0)]);
        assert_eq!(line((0, 0), (2, 2)), [(0, 0), (1, 1), (2, 2)]);
        assert_eq!(
            line((0, 0), (4, 2)),
            [(0, 0), (1, 1), (2, 1), (3, 2), (4, 2)]
        );
        // Traced from the smaller y, so both directions cover the same tiles.
        let mut back = line((4, 2), (0, 0));
        back.reverse();
        assert_eq!(back, line((0, 0), (4, 2)));
        assert_eq!(line((3, 0), (0, 1)), [(3, 0), (2, 0), (1, 1), (0, 1)]);
        assert_eq!(line((1, 1), (1, 1)), [(1, 1)]);
    }

    #[test]
    fn checks_in_order() {
        let open = |_: (i32, i32), _: (i32, i32)| false;
        assert_eq!(check((0, 0), 100, &RIFLE, &[(0, 0)], open), Fire::Invalid);
        let unarmed = Weapon {
            has_weapon: false,
            ..RIFLE
        };
        assert_eq!(
            check((0, 0), 100, &unarmed, &[(9, 9)], open),
            Fire::NoWeapon
        );
        assert_eq!(
            check((0, 0), 39, &RIFLE, &[(9, 9)], open),
            Fire::NoActionPoints
        );
        assert_eq!(check((0, 0), 40, &RIFLE, &[(6, 2)], open), Fire::OutOfRange);
        assert_eq!(
            check((0, 0), 40, &RIFLE, &[(5, 2)], open),
            Fire::Clear((5, 2))
        );
        let mortar = Weapon {
            min_range: 3,
            indirect: true,
            ..RIFLE
        };
        assert_eq!(check((0, 0), 40, &mortar, &[(3, 1)], open), Fire::TooClose);
        assert_eq!(
            check((0, 0), 40, &mortar, &[(4, 1)], |_, _| true),
            Fire::Clear((4, 1))
        );
    }

    #[test]
    fn sight_ignores_both_ends() {
        let wall = |p: (i32, i32), _: (i32, i32)| p == (2, 0);
        assert_eq!(check((0, 0), 40, &RIFLE, &[(4, 0)], wall), Fire::Blocked);
        assert_eq!(
            check((0, 0), 40, &RIFLE, &[(2, 0)], wall),
            Fire::Clear((2, 0))
        );
        assert_eq!(
            check((2, 0), 40, &RIFLE, &[(4, 0)], wall),
            Fire::Clear((4, 0))
        );
        // A wide target is hit through any tile with a clear line.
        let footprint = [(4, 0), (4, 1)];
        assert_eq!(
            check((0, 0), 40, &RIFLE, &footprint, wall),
            Fire::Clear((4, 1))
        );
        // Partly out of range and blocked: reported as out of range.
        let far = [(4, 0), (6, 0)];
        assert_eq!(check((0, 0), 40, &RIFLE, &far, wall), Fire::OutOfRange);
    }
}
