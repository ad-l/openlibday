//! Movement glue between scenario data and the `libday-sim` path finder,
//! plus the Smacker walk animations (`ANMSUNIT.FF`).

use std::collections::BTreeMap;

use libday_data::{Definitions, Scenario};
use libday_formats::{Mff, Smacker};
pub use libday_sim::path::{
    Access, MoveGrid, Movement, Step, Tile, affordable_steps, executable_steps, move_budget,
};

use crate::Image;

/// Walk animation frame rate (the Smacker files' 66 667 µs per frame).
pub const ANIMATION_FPS: f64 = 15.0;
/// Time each queued walk position is shown: one per animation frame. A
/// recording of the original advances exactly one position every ~66.5 ms.
pub const WALK_POSITION_MS: u64 = 67;

/// Positions a walking unit is drawn at between two tile centres, as the
/// original queues them (`0x46b552`): every `pix_speed`-th point of the
/// Bresenham line from `from` to `to`, excluding both ends.
pub fn walk_positions(from: (i32, i32), to: (i32, i32), pix_speed: i32) -> Vec<(i32, i32)> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let (sx, sy) = (dx.signum(), dy.signum());
    let (ax, ay) = (dx.abs(), dy.abs());
    let major = ax.max(ay);
    let step = pix_speed.max(1);
    let (mut x, mut y) = from;
    let mut err = 0;
    let mut out = Vec::new();
    for i in 1..major {
        if ax >= ay {
            x += sx;
            err += 2 * ay;
            if err > ax {
                y += sy;
                err -= 2 * ax;
            }
        } else {
            y += sy;
            err += 2 * ax;
            if err > ay {
                x += sx;
                err -= 2 * ay;
            }
        }
        if i % step == 0 {
            out.push((x, y));
        }
    }
    out
}

/// Walk path for unit `mover` (index into `scenario.units`), as its owner
/// sees the map: other units, buildings, walls, solid ruins and the armed
/// mines the owner knows about block it. `None` for units without a
/// definition or larger than one tile, which cannot move.
pub fn move_grid(scenario: &Scenario, defs: &Definitions, mover: usize) -> Option<MoveGrid> {
    let unit = scenario.units.get(mover)?;
    let kind = defs.unit(unit.kind)?;
    if kind.width != 1 || kind.height != 1 {
        return None;
    }
    let mut tiles: BTreeMap<(i64, i64), Tile> = BTreeMap::new();
    for road in &scenario.roads {
        let kind = road.kind.clamp(0, 1) as u8;
        let tile = tiles.entry((road.x, road.y)).or_default();
        tile.road = Some(tile.road.map_or(kind, |r| r.max(kind)));
    }
    let mut occupy = |x: i64, y: i64, w: i64, h: i64| {
        for dy in 0..h {
            for dx in 0..w {
                tiles.entry((x + dx, y + dy)).or_default().occupied = true;
            }
        }
    };
    for (i, u) in scenario.units.iter().enumerate() {
        // Reinforcements are not on the map yet.
        if i == mover || u.arrival != 0 {
            continue;
        }
        let (w, h) = defs.unit(u.kind).map_or((1, 1), |k| (k.width, k.height));
        occupy(u.x, u.y, w, h);
    }
    for b in &scenario.buildings {
        let (w, h) = defs
            .buildings
            .get(&b.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        occupy(b.x, b.y, w, h);
    }
    for w in &scenario.walls {
        occupy(w.x, w.y, 1, 1);
    }
    for r in &scenario.ruins {
        if defs.ruins.get(&r.id).is_some_and(|k| !k.can_move_through) {
            occupy(r.x, r.y, 1, 1);
        }
    }
    for m in &scenario.mines {
        let hidden = match unit.owner {
            0 => m.hidden_ai,
            p => m.hidden.get(p as usize - 1).copied().unwrap_or(true),
        };
        if m.armed && !hidden {
            let tile = tiles.entry((m.x, m.y)).or_default();
            tile.mine = Some(tile.mine.map_or(m.kind as u8, |k| k.max(m.kind as u8)));
        }
    }
    let size = scenario.size as i32;
    Some(MoveGrid::new(
        size,
        size,
        Movement::from_id(kind.movement),
        |x, y| {
            let (x, y) = (i64::from(x), i64::from(y));
            let mut tile = tiles.get(&(x, y)).copied().unwrap_or_default();
            tile.kind = scenario
                .cell(x, y)
                .and_then(|c| defs.terrain(&c.terrain))
                .map_or(libday_sim::path::kind::MOUNTAIN, |t| t.kind);
            tile
        },
    ))
}

/// `ANMSUNIT.FF` member of a unit's walk animation for an owner colour.
pub fn walk_animation_name(image: &str, owner: i64, facing: u8) -> String {
    let image = image.to_ascii_uppercase();
    let owner = owner.clamp(0, 4);
    format!("{image}\\P{owner}\\{image}{owner}M{facing}.SMK")
}

/// Decodes an animation to RGBA frames, with the background key colour (the
/// first frame's most frequent colour) transparent.
pub fn animation_frames(ff: &[u8], name: &str) -> Option<Vec<Image>> {
    let mff = Mff::parse(ff).ok()?;
    let smk = Smacker::parse(mff.get(name).ok()?).ok()?;
    let frames = smk.decode_all().ok()?;
    let key = frames.first()?.dominant_colour();
    let (width, height) = (smk.header.width as usize, smk.header.height as usize);
    Some(
        frames
            .iter()
            .map(|f| {
                let mut rgba = Vec::with_capacity(width * height * 4);
                for &p in &f.pixels {
                    let c = f.palette[p as usize];
                    rgba.extend_from_slice(&c);
                    rgba.push(if c == key { 0 } else { 255 });
                }
                Image {
                    width,
                    height,
                    rgba,
                }
            })
            .collect(),
    )
}

/// Path dot sprite (`SYMBOLS.DBI`) for a path step: red at the destination,
/// blue on tiles the unit may stop on, dark blue on fly-over tiles; crossed
/// once the unit cannot afford the step.
pub fn path_dot(step: &Step, last: bool, affordable: bool) -> &'static str {
    match (last, step.stop, affordable) {
        (true, _, true) => "PATH0004",
        (true, _, false) => "PATH0005",
        (false, true, true) => "PATH0000",
        (false, true, false) => "PATH0001",
        (false, false, true) => "PATH0002",
        (false, false, false) => "PATH0003",
    }
}

#[cfg(test)]
mod tests {
    use super::walk_positions;

    #[test]
    fn walk_positions_sample_every_pix_speed_points() {
        assert_eq!(
            walk_positions((0, 0), (27, 14), 8),
            vec![(8, 4), (16, 8), (24, 12)]
        );
        assert_eq!(walk_positions((0, 0), (54, 0), 8).len(), 6);
        assert_eq!(
            walk_positions((10, 10), (10, -18), 8),
            vec![(10, 2), (10, -6), (10, -14)]
        );
    }
}
