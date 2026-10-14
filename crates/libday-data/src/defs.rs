//! Typed game definitions from `GLOBALS\GLOBAL.MDB`.
//!
//! Only the fields the engine currently uses are typed; the tables hold many
//! more columns, which stay readable through [`crate::records::Records`].

use std::collections::BTreeMap;

use libday_formats::Database;

use crate::DataError;
use crate::records::Records;

/// `Terrain Types.type`.
pub mod terrain_kind {
    pub const SHALLOW_WATER: u8 = 0;
    pub const DEEP_WATER: u8 = 1;
    pub const RIVERBANK: u8 = 2;
    pub const PLAIN: u8 = 3;
    pub const MOUNTAIN: u8 = 4;
    pub const TREE: u8 = 5;
    pub const CRACK: u8 = 6;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainType {
    /// Artwork identifier in `TERRAINS.DBI` (upper case), also the type key.
    pub id: String,
    pub kind: u8,
    pub overlapping: bool,
    /// Average colour (minimap).
    pub colour: [u8; 3],
    pub burned_id: String,
    pub can_fire_through: bool,
}

impl TerrainType {
    /// Plain ground that can be painted freely.
    pub fn paintable(&self) -> bool {
        self.kind == terrain_kind::PLAIN && !self.overlapping
    }

    pub fn tree(&self) -> bool {
        self.kind == terrain_kind::TREE
    }

    /// Non-paintable land decoration (rocks, cliffs, overlapping plain tiles).
    pub fn scenery(&self) -> bool {
        matches!(self.kind, 3 | 4 | 6) && !self.paintable()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildingType {
    pub id: i64,
    pub name: String,
    pub width: i64,
    pub height: i64,
    /// Required terrain kind of every footprint tile (1 water, 3 plain).
    pub position: i64,
    pub armor: i64,
    pub unique: bool,
    pub image: String,
    /// Unit type the building turns into, if any.
    pub transform_into: Option<i64>,
    pub ai_target: bool,
    pub damaged_images: bool,
    pub category: i64,
}

impl BuildingType {
    /// `UB.DBI` sprite for footprint part `(dx, dy)`.
    pub fn sprite(&self, dx: i64, dy: i64, damaged: bool, level: u8) -> String {
        let damaged = damaged && self.damaged_images && self.transform_into.is_none();
        format!("{}{level}{}{dx}{dy}", self.image, u8::from(damaged))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitType {
    pub id: i64,
    pub tech: i64,
    pub name: String,
    pub race: i64,
    pub facings: bool,
    pub width: i64,
    pub height: i64,
    pub movement: i64,
    pub armor: i64,
    pub action_points: i64,
    /// AP of one shot (`actionPointsShoot`), reserved by overwatch.
    pub shoot_action_points: i64,
    pub can_overwatch: bool,
    /// Screen pixels advanced per animation frame while moving.
    pub move_pix_speed: i64,
    /// Has a walk animation (`MoveAnimation_Begin` != -1).
    pub move_animation: bool,
    pub image: String,
}

impl UnitType {
    /// `UB.DBI` sprite for footprint part `(dx, dy)`.
    pub fn sprite(&self, dx: i64, dy: i64, facing: i64, frame: u8) -> String {
        let facing = if self.facings { facing } else { 0 };
        format!("{}{frame}{facing}{dx}{dy}", self.image)
    }

    pub fn facing_count(&self) -> i64 {
        if self.facings { 8 } else { 1 }
    }
}

/// Road or wall type (`Road Types`, `Wall Types`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceType {
    pub id: i64,
    pub name: String,
    pub armor: i64,
    /// `ROADS.DBI` / `WALLS.DBI` prefix.
    pub image: String,
}

/// Wreckage/debris type (`Ruin Types`); the id is its `RUINS.DBI` sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuinType {
    pub id: String,
    pub can_fire_through: bool,
    pub can_move_through: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MineType {
    pub id: i64,
    pub name: String,
    /// `UB.DBI` prefix.
    pub image: String,
    pub damage_base: i64,
    pub damage_random: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Definitions {
    pub terrains: BTreeMap<String, TerrainType>,
    pub buildings: BTreeMap<i64, BuildingType>,
    /// Keyed by `(type, technology level)`.
    pub units: BTreeMap<(i64, i64), UnitType>,
    pub roads: BTreeMap<i64, SurfaceType>,
    pub walls: BTreeMap<i64, SurfaceType>,
    pub ruins: BTreeMap<String, RuinType>,
    pub mines: BTreeMap<i64, MineType>,
}

fn invalid(table: &str, message: String) -> DataError {
    DataError::Invalid {
        table: table.to_owned(),
        message,
    }
}

fn insert_unique<K: Ord + std::fmt::Debug, V>(
    map: &mut BTreeMap<K, V>,
    table: &str,
    key: K,
    value: V,
) -> Result<(), DataError> {
    if map.contains_key(&key) {
        return Err(invalid(table, format!("duplicate key {key:?}")));
    }
    map.insert(key, value);
    Ok(())
}

impl Definitions {
    pub fn load(global: &Database) -> Result<Self, DataError> {
        let mut defs = Self::default();

        let t = Records::read(global, "Terrain Types")?;
        for r in 0..t.len() {
            let id = t.text(r, "id")?.to_ascii_uppercase();
            let mut colour = [0u8; 3];
            for (channel, column) in
                colour
                    .iter_mut()
                    .zip(["Color_Red", "Color_Green", "Color_Blue"])
            {
                *channel = t.int_or(r, column, 0)?.clamp(0, 255) as u8;
            }
            let item = TerrainType {
                id: id.clone(),
                kind: t.int(r, "type")? as u8,
                overlapping: t.flag(r, "Overlapping")?,
                colour,
                burned_id: t.text_or(r, "burned_id")?.to_ascii_uppercase(),
                can_fire_through: t.flag_or(r, "can_fire_through", true)?,
            };
            insert_unique(&mut defs.terrains, &t.table, id, item)?;
        }

        let t = Records::read(global, "Building Types")?;
        for r in 0..t.len() {
            let transform = t.int(r, "TransformIntoUnit")?;
            let item = BuildingType {
                id: t.int(r, "TypeID")?,
                name: t.text(r, "NameEnglish")?,
                width: t.int(r, "SizeX")?,
                height: t.int(r, "SizeY")?,
                position: t.int(r, "Position")?,
                armor: t.int(r, "Armor")?,
                unique: t.flag(r, "UniqueOnMap")?,
                image: t.text(r, "ImageID")?.to_ascii_uppercase(),
                transform_into: (transform != 65535).then_some(transform),
                ai_target: t.flag(r, "isAItarget")?,
                damaged_images: t.flag(r, "DamagedImages")?,
                category: t.int_or(r, "BuildingCat", 0)?,
            };
            if !(1..=9).contains(&item.width) || !(1..=9).contains(&item.height) {
                return Err(invalid(&t.table, format!("type {}: bad size", item.id)));
            }
            insert_unique(&mut defs.buildings, &t.table, item.id, item)?;
        }

        let t = Records::read(global, "Unit Types")?;
        for r in 0..t.len() {
            let item = UnitType {
                id: t.int(r, "typeID")?,
                tech: t.int(r, "unitTechLevel")?,
                name: t.text(r, "NameEnglish")?,
                race: t.int(r, "Race")?,
                facings: t.flag(r, "Facings")?,
                width: t.int(r, "SizeX")?,
                height: t.int(r, "SizeY")?,
                movement: t.int(r, "Movement")?,
                armor: t.int(r, "Armor")?,
                action_points: t.int(r, "actionPoints")?,
                shoot_action_points: t.int_or(r, "actionPointsShoot", 0)?,
                can_overwatch: t.flag_or(r, "canOverwatch", false)?,
                move_pix_speed: t.int_or(r, "MoveAnimation_PixSpeed", 8)?,
                move_animation: t.int_or(r, "MoveAnimation_Begin", -1)? != -1,
                image: t.text(r, "facingsImageID")?.to_ascii_uppercase(),
            };
            if !(1..=9).contains(&item.width) || !(1..=9).contains(&item.height) {
                return Err(invalid(&t.table, format!("type {}: bad size", item.id)));
            }
            insert_unique(&mut defs.units, &t.table, (item.id, item.tech), item)?;
        }

        for (table, id, armor, target) in [
            ("Road Types", "RoadType", "ar", &mut defs.roads),
            ("Wall Types", "WallType", "Armor", &mut defs.walls),
        ] {
            let t = Records::read(global, table)?;
            for r in 0..t.len() {
                let item = SurfaceType {
                    id: t.int(r, id)?,
                    name: t.text(r, "nameEnglish")?,
                    armor: t.int(r, armor)?,
                    image: t.text(r, "img_id")?.to_ascii_uppercase(),
                };
                insert_unique(target, table, item.id, item)?;
            }
        }

        let t = Records::read(global, "Ruin Types")?;
        for r in 0..t.len() {
            let item = RuinType {
                id: t.text(r, "id")?.to_ascii_uppercase(),
                can_fire_through: t.flag(r, "can_fire_through")?,
                can_move_through: t.flag(r, "can_move_through")?,
            };
            insert_unique(&mut defs.ruins, &t.table, item.id.clone(), item)?;
        }

        let t = Records::read(global, "Mine Types")?;
        for r in 0..t.len() {
            let item = MineType {
                id: t.int(r, "typeID")?,
                name: t.text(r, "nameEnglish")?,
                image: t.text(r, "ImageID")?.to_ascii_uppercase(),
                damage_base: t.int_or(r, "damageBase", 0)?,
                damage_random: t.int_or(r, "damageRandom", 0)?,
            };
            insert_unique(&mut defs.mines, &t.table, item.id, item)?;
        }
        Ok(defs)
    }

    pub fn terrain(&self, id: &str) -> Option<&TerrainType> {
        self.terrains.get(&id.to_ascii_uppercase())
    }

    /// Base (technology level 0) definition of a unit type.
    pub fn unit(&self, id: i64) -> Option<&UnitType> {
        self.units.get(&(id, 0))
    }
}
