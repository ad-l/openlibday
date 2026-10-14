//! Shot animations (`ANMSUNIT.FF`): the shooter's raise/fire/lower
//! sequences, the projectile's flight and the impact at the target.

use libday_formats::{Mff, Smacker};

/// Animation kinds, by their letter in the member name (the original's
/// table at `0x4ec730`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    /// `S`: raise the weapon.
    Begin,
    /// `T`: fire.
    Shoot,
    /// `U`: lower the weapon.
    End,
    /// `P`: the projectile.
    Projectile,
    /// `I`: impact at the target.
    Impact,
    /// `E`: the unit blows up.
    Explosion,
}

impl Anim {
    fn letter(self) -> char {
        match self {
            Anim::Begin => 'S',
            Anim::Shoot => 'T',
            Anim::End => 'U',
            Anim::Projectile => 'P',
            Anim::Impact => 'I',
            Anim::Explosion => 'E',
        }
    }
}

/// `ANMSUNIT.FF` member of a shot animation. Projectile, impact and
/// explosion animations have one facing (`0x43650b`).
pub fn animation_name(image: &str, owner: i64, anim: Anim, facing: u8) -> String {
    let image = image.to_ascii_uppercase();
    let owner = owner.clamp(0, 4);
    let facing = match anim {
        Anim::Projectile | Anim::Impact | Anim::Explosion => 0,
        _ => facing,
    };
    format!(
        "{image}\\P{owner}\\{image}{owner}{}{facing}.SMK",
        anim.letter()
    )
}

/// Frame count of an animation, from its header.
pub fn animation_length(ff: &[u8], name: &str) -> Option<usize> {
    let mff = Mff::parse(ff).ok()?;
    let smk = Smacker::parse(mff.get(name).ok()?).ok()?;
    Some(smk.header.frames as usize)
}

/// Pixels of the line from `a` to `b`, both included.
fn pixel_line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let (mut x, mut y) = a;
    let mut err = dx + dy;
    let mut out = vec![a];
    while (x, y) != b {
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        out.push((x, y));
    }
    out
}

/// Positions of a projectile flying from `from` to `to` (map pixels), one
/// per animation frame, as the original queues them (`0x46c797`): the
/// pixel line between the two, lifted into an arc by `sin(kπ/n)` times
/// `curve` percent of its length `n` (`0x46c599`), less the first
/// `pix_offset` points (`0x46c673`), then thinned to points at least
/// `pix_speed` pixels apart (`0x46c6f9`).
pub fn projectile_path(
    from: (i32, i32),
    to: (i32, i32),
    pix_offset: i64,
    pix_speed: i64,
    curve: i64,
) -> Vec<(i32, i32)> {
    let mut points = pixel_line(from, to);
    let n = points.len() as f64;
    if curve != 0 {
        for (k, p) in points.iter_mut().enumerate() {
            let lift = ((k + 1) as f64 * std::f64::consts::PI / n).sin() * n * curve as f64 * 0.01;
            p.1 -= lift as i32;
        }
    }
    let skip = (pix_offset.max(0) as usize).min(points.len());
    let points = &points[skip..];
    let mut out: Vec<(i32, i32)> = Vec::new();
    for &p in points {
        match out.last() {
            Some(&(x, y)) if i64::from((p.0 - x).abs().max((p.1 - y).abs())) < pix_speed => {}
            _ => out.push(p),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_use_facing_zero_for_projectiles() {
        assert_eq!(
            animation_name("hltk", 2, Anim::Shoot, 5),
            "HLTK\\P2\\HLTK2T5.SMK"
        );
        assert_eq!(
            animation_name("HLTK", 2, Anim::Projectile, 5),
            "HLTK\\P2\\HLTK2P0.SMK"
        );
    }

    #[test]
    fn straight_projectile_is_thinned() {
        let path = projectile_path((0, 0), (100, 0), 10, 20, 0);
        assert_eq!(path, vec![(10, 0), (30, 0), (50, 0), (70, 0), (90, 0)]);
    }

    #[test]
    fn curved_projectile_arcs_up() {
        let path = projectile_path((0, 0), (100, 0), 0, 1, 10);
        let top = path.iter().map(|p| p.1).min().unwrap();
        assert_eq!(top, -10);
        assert_eq!(path.last(), Some(&(100, 0)));
    }
}
