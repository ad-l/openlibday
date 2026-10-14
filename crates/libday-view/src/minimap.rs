//! Top-down map pictures for the sidebar minimap and the MAP overview, plus
//! the per-player unit lists of the overview.

use libday_data::{Definitions, Scenario};

use crate::scene::{Layer, Scene};
use crate::{Art, Image};

/// Player colours by owner: the AI (owner 0) is red, then the four human
/// players in swatch order (blue, yellow, magenta, green).
pub const OWNER_COLOURS: [[u8; 3]; 5] = [
    [224, 0, 0],
    [0, 72, 255],
    [255, 224, 0],
    [255, 0, 255],
    [0, 216, 0],
];

pub fn owner_colour(owner: i64) -> [u8; 3] {
    OWNER_COLOURS[owner.clamp(0, 4) as usize]
}

/// Darker shade used for buildings.
pub fn building_colour(owner: i64) -> [u8; 3] {
    owner_colour(owner).map(|c| (c as u16 * 3 / 5) as u8)
}

/// First frame of an owner's colour swatch in the interface artwork.
pub fn swatch_frame(owner: i64) -> u16 {
    crate::ui::frame::SWATCH + 6 * owner.clamp(0, 4) as u16
}

pub fn player_name(owner: i64) -> String {
    if owner == 0 {
        "Alien".to_owned()
    } else {
        format!("Player {owner}")
    }
}

/// A unit or building shown on the minimap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub owner: i64,
    pub building: bool,
    pub kind: i64,
}

/// Buildings and the units on the map at the start (no reinforcements).
pub fn markers(scenario: &Scenario, defs: &Definitions) -> Vec<Marker> {
    let mut markers = Vec::new();
    for b in &scenario.buildings {
        let (width, height) = defs
            .buildings
            .get(&b.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        markers.push(Marker {
            x: b.x,
            y: b.y,
            width,
            height,
            owner: b.owner,
            building: true,
            kind: b.kind,
        });
    }
    for u in scenario.units.iter().filter(|u| u.arrival == 0) {
        let (width, height) = defs.unit(u.kind).map_or((1, 1), |k| (k.width, k.height));
        markers.push(Marker {
            x: u.x,
            y: u.y,
            width,
            height,
            owner: u.owner,
            building: false,
            kind: u.kind,
        });
    }
    markers
}

/// Owners the overview lists: human players with a starting zone or
/// anything on the map, then the AI.
pub fn players(scenario: &Scenario, markers: &[Marker]) -> Vec<i64> {
    let mut players: Vec<i64> = (1..=4)
        .filter(|&p| {
            scenario
                .deployments
                .iter()
                .any(|d| d.player == p && d.is_start_zone())
                || markers.iter().any(|m| m.owner == p)
        })
        .collect();
    players.push(0);
    players
}

/// One overview list line: "Name xCount".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub count: usize,
    pub building: bool,
    pub kind: i64,
}

impl Entry {
    pub fn label(&self) -> String {
        format!("{} x{}", self.name, self.count)
    }

    pub fn matches(&self, marker: &Marker) -> bool {
        marker.building == self.building && marker.kind == self.kind
    }
}

/// An owner's units, then buildings, grouped by type in map order.
pub fn entries(markers: &[Marker], defs: &Definitions, owner: i64) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for building in [false, true] {
        for m in markers
            .iter()
            .filter(|m| m.owner == owner && m.building == building)
        {
            if let Some(entry) = entries.iter_mut().find(|e| e.matches(m)) {
                entry.count += 1;
                continue;
            }
            let name = if building {
                defs.buildings.get(&m.kind).map(|k| k.name.clone())
            } else {
                defs.unit(m.kind).map(|k| k.name.clone())
            };
            entries.push(Entry {
                name: name.unwrap_or_else(|| format!("#{}", m.kind)),
                count: 1,
                building,
                kind: m.kind,
            });
        }
    }
    entries
}

const ROAD: [u8; 3] = [150, 150, 150];
const WALL: [u8; 3] = [96, 96, 104];

/// One pixel per tile: the terrain's average colour, with roads and walls.
pub fn tile_colours(scenario: &Scenario, defs: &Definitions) -> Image {
    let size = scenario.size;
    let mut image = Image::new(size, size, [0, 0, 0, 255]);
    let mut set = |x: i64, y: i64, c: [u8; 3]| {
        if scenario.contains(x, y) {
            let at = (y as usize * size + x as usize) * 4;
            image.rgba[at..at + 3].copy_from_slice(&c);
        }
    };
    for y in 0..size as i64 {
        for x in 0..size as i64 {
            if let Some(t) = scenario.cell(x, y).and_then(|c| defs.terrain(&c.terrain)) {
                set(x, y, t.colour);
            }
        }
    }
    for r in &scenario.roads {
        set(r.x, r.y, ROAD);
    }
    for w in scenario.walls.iter().filter(|w| !w.broken) {
        set(w.x, w.y, WALL);
    }
    image
}

/// A `cell`x`cell`-pixel-per-tile top-down picture, sampling the diamond of
/// each terrain, road, decoration and wall sprite (columns are tile x, rows
/// tile y); buildings and units are left to the markers.
pub fn top_down(scene: &Scene, art: &Art, cell: usize) -> Image {
    let size = scene.size;
    let cell = cell.max(1);
    let side = size * cell;
    let mut image = Image::new(side, side, [0, 0, 0, 255]);
    let mut cache = std::collections::HashMap::new();
    for item in &scene.items {
        if !matches!(
            item.layer,
            Layer::Terrain | Layer::Road | Layer::Decor | Layer::Wall
        ) {
            continue;
        }
        let sprite = cache
            .entry((item.sheet, item.sprite.as_str()))
            .or_insert_with(|| art.rgba(item.sheet, &item.sprite));
        let Some((w, h, rgba)) = sprite else { continue };
        if !(0..size as i64).contains(&item.x) || !(0..size as i64).contains(&item.y) {
            continue;
        }
        for row in 0..cell {
            for col in 0..cell {
                // u runs along tile x (towards the right corner), v along y.
                let u = (col as f32 + 0.5) / cell as f32;
                let v = (row as f32 + 0.5) / cell as f32;
                let sx = (27.0 + 27.0 * (u - v)) as usize;
                let sy = (28.0 + 14.0 * (u + v)) as usize;
                if sx >= *w || sy >= *h {
                    continue;
                }
                let at = (sy * *w + sx) * 4;
                if rgba[at + 3] == 0 {
                    continue;
                }
                let px = item.x as usize * cell + col;
                let py = item.y as usize * cell + row;
                let to = (py * side + px) * 4;
                image.rgba[to..to + 3].copy_from_slice(&rgba[at..at + 3]);
            }
        }
    }
    image
}

/// Box-filtered resize (for shrinking without aliasing).
pub fn downscale(image: &Image, width: usize, height: usize) -> Image {
    let mut out = Image::new(width.max(1), height.max(1), [0, 0, 0, 255]);
    let (width, height) = (out.width, out.height);
    for y in 0..height {
        let (y0, y1) = span(y, height, image.height);
        for x in 0..width {
            let (x0, x1) = span(x, width, image.width);
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let at = (sy * image.width + sx) * 4;
                    for (s, &c) in sum.iter_mut().zip(&image.rgba[at..at + 4]) {
                        *s += c as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as u32;
            let at = (y * width + x) * 4;
            for (o, s) in out.rgba[at..at + 4].iter_mut().zip(sum) {
                *o = (s / n) as u8;
            }
        }
    }
    out
}

/// Source range covered by destination pixel `i` of `n` over `len` pixels.
fn span(i: usize, n: usize, len: usize) -> (usize, usize) {
    let a = i * len / n;
    let b = ((i + 1) * len / n).max(a + 1).min(len);
    (a.min(len.saturating_sub(1)), b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(owner: i64, building: bool, kind: i64) -> Marker {
        Marker {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            owner,
            building,
            kind,
        }
    }

    #[test]
    fn groups_units_before_buildings() {
        let defs = Definitions::default();
        let markers = [
            marker(0, true, 7),
            marker(0, false, 3),
            marker(1, false, 3),
            marker(0, false, 3),
            marker(0, false, 4),
        ];
        let list = entries(&markers, &defs, 0);
        let labels: Vec<String> = list.iter().map(Entry::label).collect();
        assert_eq!(labels, ["#3 x2", "#4 x1", "#7 x1"]);
        assert!(list[2].building && list[2].matches(&markers[0]));
        assert_eq!(entries(&markers, &defs, 2), []);
    }

    #[test]
    fn owner_colours_and_swatches() {
        assert_eq!(owner_colour(0), [224, 0, 0]);
        assert_eq!(owner_colour(9), owner_colour(4));
        assert_eq!(swatch_frame(1), 52);
        assert_eq!(player_name(0), "Alien");
        assert_eq!(player_name(2), "Player 2");
    }

    #[test]
    fn downscale_averages() {
        let mut image = Image::new(4, 2, [0, 0, 0, 255]);
        image.rgba[0..4].copy_from_slice(&[200, 100, 40, 255]);
        let small = downscale(&image, 2, 1);
        assert_eq!(&small.rgba[0..4], &[50, 25, 10, 255]);
        assert_eq!(&small.rgba[4..8], &[0, 0, 0, 255]);
        assert_eq!(downscale(&image, 8, 4).width, 8);
    }
}
