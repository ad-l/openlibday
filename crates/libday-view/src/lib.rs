//! Map scene building for the tactical view, independent of any window
//! system: artwork loading, the isometric projection, the ordered draw list
//! of a scenario and a software compositor (used for PNG export and tests).
//!
//! The draw order reproduces the modding map editor
//! (`tools/modding/map_editor.py` `render_map`), which is the reference.

pub mod compose;
pub mod damage;
pub mod fire;
pub mod mines;
pub mod minimap;
pub mod movement;
pub mod scatter;
pub mod scene;
pub mod shot;
pub mod text;
pub mod ui;

use std::collections::BTreeMap;

use libday_data::defs::UnitType;
use libday_data::{DataError, Definitions, Dir, GameData};
use libday_formats::{Dbi, Sprite};

pub use compose::{Image, compose};
pub use scene::{DrawItem, Layer, Options, Scene, build};

/// Tile footprint: a 54x28 diamond; every sprite is a 54x56 cell whose
/// diamond sits 42 pixels below the cell top.
pub const TILE_W: i32 = 54;
pub const TILE_H: i32 = 28;
pub const SPRITE_W: i32 = 54;
pub const SPRITE_H: i32 = 56;
pub const SPRITE_ANCHOR_Y: i32 = 42;

/// Artwork database (`IMGS\*.DBI`) a sprite comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sheet {
    Terrains,
    Ub,
    Roads,
    Walls,
    Ruins,
    Symbols,
}

impl Sheet {
    pub const ALL: [Sheet; 6] = [
        Sheet::Terrains,
        Sheet::Ub,
        Sheet::Roads,
        Sheet::Walls,
        Sheet::Ruins,
        Sheet::Symbols,
    ];

    pub fn file(self) -> &'static str {
        match self {
            Sheet::Terrains => "TERRAINS.DBI",
            Sheet::Ub => "UB.DBI",
            Sheet::Roads => "ROADS.DBI",
            Sheet::Walls => "WALLS.DBI",
            Sheet::Ruins => "RUINS.DBI",
            Sheet::Symbols => "SYMBOLS.DBI",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sheet::Terrains => "terrains",
            Sheet::Ub => "ub",
            Sheet::Roads => "roads",
            Sheet::Walls => "walls",
            Sheet::Ruins => "ruins",
            Sheet::Symbols => "symbols",
        }
    }
}

/// The map artwork plus the per-type artwork choices the view needs.
pub struct Art {
    sheets: BTreeMap<Sheet, Dbi<'static>>,
    /// Animation frame shown for a standing unit of each type.
    pub unit_frames: BTreeMap<i64, u8>,
    /// Complete owner-colour frames of each unit type (0 = alien/neutral,
    /// 1-4 = players 1-4).
    pub owner_frames: BTreeMap<i64, Vec<u8>>,
    /// Construction level shown for each building type.
    pub building_levels: BTreeMap<i64, u8>,
}

impl Art {
    pub fn load(game: &GameData, defs: &Definitions) -> Result<Self, DataError> {
        let mut sheets = BTreeMap::new();
        for sheet in Sheet::ALL {
            let bytes = game.read(Dir::Imgs, sheet.file())?;
            let dbi = Dbi::from_vec(bytes).map_err(|e| DataError::Invalid {
                table: sheet.file().to_owned(),
                message: e.to_string(),
            })?;
            sheets.insert(sheet, dbi);
        }
        Ok(Self::new(sheets, defs))
    }

    pub fn new(sheets: BTreeMap<Sheet, Dbi<'static>>, defs: &Definitions) -> Self {
        let mut art = Self {
            sheets,
            unit_frames: BTreeMap::new(),
            owner_frames: BTreeMap::new(),
            building_levels: BTreeMap::new(),
        };
        let has = |art: &Art, name: &str| art.contains(Sheet::Ub, name);
        let mut frames = BTreeMap::new();
        let mut owner_frames = BTreeMap::new();
        for ((id, tech), kind) in &defs.units {
            if *tech != 0 {
                continue;
            }
            let complete = |frame: u8| {
                (0..kind.facing_count()).all(|facing| {
                    footprint(kind.width, kind.height)
                        .all(|(dx, dy)| has(&art, &kind.sprite(dx, dy, facing, frame)))
                })
            };
            if let Some(frame) = (0..10u8).find(|&frame| complete(frame)) {
                frames.insert(*id, frame);
            }
            let owners: Vec<u8> = (0..5u8).filter(|&frame| complete(frame)).collect();
            if !owners.is_empty() {
                owner_frames.insert(*id, owners);
            }
        }
        art.owner_frames = owner_frames;
        let mut levels = BTreeMap::new();
        for (id, kind) in &defs.buildings {
            let complete = |level: u8| {
                footprint(kind.width, kind.height)
                    .all(|(dx, dy)| has(&art, &kind.sprite(dx, dy, false, level)))
            };
            // Defence towers show their deployed (upgraded) artwork.
            let preferred = match id {
                131 => Some(1),
                173 => Some(2),
                174 => Some(3),
                175 => Some(4),
                _ => None,
            };
            let level = preferred
                .filter(|&level| complete(level))
                .or_else(|| (0..5u8).find(|&level| complete(level)));
            if let Some(level) = level {
                levels.insert(*id, level);
            }
        }
        art.unit_frames = frames;
        art.building_levels = levels;
        art
    }

    /// Frame drawn for a unit of type `kind` owned by `owner`: the owner's
    /// colour when the artwork has it, else the type's default frame.
    pub fn unit_frame(&self, kind: i64, owner: i64) -> Option<u8> {
        let owned = u8::try_from(owner).ok().filter(|&frame| {
            self.owner_frames
                .get(&kind)
                .is_some_and(|frames| frames.contains(&frame))
        });
        owned.or_else(|| self.unit_frames.get(&kind).copied())
    }

    /// Sprite of a foot unit hidden in a forest (`{image}{frame}{A-H}00`: an
    /// owner-coloured cone topped by the unit), if the artwork has one.
    pub fn forest_topper(&self, kind: &UnitType, facing: i64, frame: u8) -> Option<String> {
        let letter = char::from(b'A' + u8::try_from(facing.rem_euclid(8)).ok()?);
        [frame, 1, 0]
            .into_iter()
            .map(|frame| format!("{}{frame}{letter}00", kind.image))
            .find(|name| self.contains(Sheet::Ub, name))
    }

    pub fn sheet(&self, sheet: Sheet) -> Option<&Dbi<'static>> {
        self.sheets.get(&sheet)
    }

    pub fn contains(&self, sheet: Sheet, name: &str) -> bool {
        self.sheet(sheet).is_some_and(|dbi| dbi.contains(name))
    }

    pub fn sprite(&self, sheet: Sheet, name: &str) -> Option<Sprite> {
        self.sheet(sheet)?.sprite(name).ok()
    }

    /// RGBA pixels of a sprite in its sheet's palette.
    pub fn rgba(&self, sheet: Sheet, name: &str) -> Option<(usize, usize, Vec<u8>)> {
        let dbi = self.sheet(sheet)?;
        let sprite = dbi.sprite(name).ok()?;
        Some((sprite.width, sprite.height, sprite.to_rgba(&dbi.palette)))
    }
}

pub(crate) fn footprint(width: i64, height: i64) -> impl Iterator<Item = (i64, i64)> {
    (0..width).flat_map(move |dx| (0..height).map(move |dy| (dx, dy)))
}

/// Pixel size of a whole `size`x`size` map image.
pub fn map_pixels(size: usize) -> (i32, i32) {
    let size = size as i32;
    (size * TILE_W, size * TILE_H + TILE_H)
}

/// Diamond centre of tile `(x, y)` in whole-map pixel coordinates.
pub fn tile_center(size: usize, x: i64, y: i64) -> (i32, i32) {
    let (x, y) = (x as i32, y as i32);
    (
        size as i32 * TILE_W / 2 + (x - y) * TILE_W / 2,
        SPRITE_ANCHOR_Y + (x + y) * TILE_H / 2,
    )
}

/// Top-left corner of the sprite cell drawn on tile `(x, y)`.
pub fn sprite_origin(size: usize, x: i64, y: i64) -> (i32, i32) {
    let (cx, cy) = tile_center(size, x, y);
    (cx - SPRITE_W / 2, cy - SPRITE_ANCHOR_Y)
}

/// Continuous tile coordinates of whole-map pixel `(px, py)`: tile `(x, y)`
/// covers `[x, x + 1) x [y, y + 1)`. Not limited to the map.
pub fn tile_coords(size: usize, px: f64, py: f64) -> (f64, f64) {
    let (ox, oy) = tile_center(size, 0, 0);
    let dx = (px - ox as f64) / (TILE_W as f64 / 2.0);
    let dy = (py - oy as f64) / (TILE_H as f64 / 2.0);
    ((dx + dy) / 2.0 + 0.5, (dy - dx) / 2.0 + 0.5)
}

/// Whole-map pixel of continuous tile coordinates (inverse of [`tile_coords`]).
pub fn map_point(size: usize, tx: f64, ty: f64) -> (f64, f64) {
    let (ox, oy) = tile_center(size, 0, 0);
    let (u, v) = (tx - 0.5, ty - 0.5);
    (
        ox as f64 + (u - v) * TILE_W as f64 / 2.0,
        oy as f64 + (u + v) * TILE_H as f64 / 2.0,
    )
}

/// Tile whose diamond contains whole-map pixel `(px, py)`.
pub fn tile_at(size: usize, px: f64, py: f64) -> Option<(i64, i64)> {
    let (u, v) = tile_coords(size, px, py);
    let (x, y) = (u.floor() as i64, v.floor() as i64);
    ((0..size as i64).contains(&x) && (0..size as i64).contains(&y)).then_some((x, y))
}

/// Loads the battle interface artwork from the installation.
pub fn load_ui(game: &GameData) -> Result<ui::UiArt, DataError> {
    let bytes = game.read(Dir::Interf, ui::FILE)?;
    ui::UiArt::from_smacker(&bytes).map_err(|e| DataError::Invalid {
        table: ui::FILE.to_owned(),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_round_trips() {
        let size = 40;
        assert_eq!(sprite_origin(size, 0, 0), (39 * 27, 0));
        assert_eq!(map_pixels(size), (2160, 1148));
        for x in 0..size as i64 {
            for y in 0..size as i64 {
                let (cx, cy) = tile_center(size, x, y);
                assert_eq!(tile_at(size, cx as f64, cy as f64), Some((x, y)));
                assert_eq!(tile_at(size, cx as f64 + 20.0, cy as f64), Some((x, y)));
            }
        }
        assert_eq!(tile_at(size, 0.0, 0.0), None);
        let (px, py) = map_point(size, 3.25, 7.5);
        let (u, v) = tile_coords(size, px, py);
        assert!((u - 3.25).abs() < 1e-9 && (v - 7.5).abs() < 1e-9);
    }
}
