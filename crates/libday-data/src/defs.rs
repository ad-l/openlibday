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
    /// `burned_min`, rounded up: the least damage that burns the tile.
    pub burned_min: i64,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    pub race: i64,
    /// `isDestructible`: takes damage from weapons.
    pub destructible: bool,
    /// `hasRuin`: leaves `B?RUI` wreckage when destroyed.
    pub has_ruin: bool,
    /// `ExplosionAnimation_Begin` and `_Qty`: frames of
    /// `BUILDING\{image}.SMK` played when destroyed.
    pub explosion: Option<(usize, usize)>,
    /// `AIcategory`: column of the AI priority table it is scored by.
    pub ai_category: i64,
    /// `Cost`: what destroying one is worth in the battle ranking.
    pub cost: i64,
    /// `TechReward`: technology found by standing next to it (tech
    /// centres and ancient ruins); 0 for every other building.
    pub tech_reward: i64,
    /// `CanBuild`: offered in the campaign base editor.
    pub can_build: bool,
    /// `Prerequisite`: building type the player must own first (65535
    /// none).
    pub prerequisite: i64,
    /// `Recycle`: credits refunded when recycled in a later base session.
    pub recycle: i64,
}

impl BuildingType {
    /// Drawn damaged at a third of its armour or less (`0x42811b`).
    pub fn damaged(&self, armor: i64) -> bool {
        armor as f64 <= self.armor as f64 * 0.33
    }

    /// `UB.DBI` sprite for footprint part `(dx, dy)`.
    pub fn sprite(&self, dx: i64, dy: i64, damaged: bool, level: u8) -> String {
        let damaged = damaged && self.damaged_images && self.transform_into.is_none();
        format!("{}{level}{}{dx}{dy}", self.image, u8::from(damaged))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitType {
    pub id: i64,
    pub tech: i64,
    pub name: String,
    pub race: i64,
    pub facings: bool,
    pub width: i64,
    pub height: i64,
    pub movement: i64,
    /// `category`: 0 hover/air, 1 ground, 2 naval.
    pub category: i64,
    pub armor: i64,
    pub action_points: i64,
    /// AP of one shot (`actionPointsShoot`), reserved by overwatch.
    pub shoot_action_points: i64,
    pub can_overwatch: bool,
    /// Screen pixels advanced per animation frame while moving.
    pub move_pix_speed: i64,
    /// Has a walk animation (`MoveAnimation_Begin` != -1).
    pub move_animation: bool,
    /// Has a take-off (`L`) animation played before moving
    /// (`MoveStartAnimation_Begin` != -1).
    pub move_start_animation: bool,
    /// Has a landing (`N`) animation played on arrival
    /// (`MoveEndAnimation_Begin` != -1).
    pub move_end_animation: bool,
    pub image: String,
    /// Deployment cost (`Cost`).
    pub cost: i64,
    /// Technology points to reach the next level (`CostToNextTL`); 0 at the
    /// maximum level.
    pub cost_to_next: i64,
    pub damage_base: i64,
    pub damage_random: i64,
    /// `damageType`: 0 destroys (armour), 1 paralyses (action points).
    pub damage_type: i64,
    /// `DamageArea`: side of the square the damage spreads over (1, 3, 5).
    pub damage_area: i64,
    /// `ruinImageID`: wreckage left on plain ground when destroyed.
    pub ruin: String,
    /// `weaponRange`.
    pub range: i64,
    /// `WeaponMinimumRange`: targets must be farther than this.
    pub min_range: i64,
    /// `UnitHasWeapon`: fires at ground targets.
    pub has_weapon: bool,
    /// `IsAA`: fires at aircraft.
    pub anti_air: bool,
    /// `DamageAA`: damage against aircraft (shown instead of the average
    /// damage for anti-aircraft units).
    pub damage_aa: i64,
    /// `MineDetectionRange`.
    pub mine_detection: i64,
    /// `weaponType` (1 = indirect fire).
    pub weapon_type: i64,
    /// `Jammer`: 0 = stealth, below 100 = reduced signature. Scales (as a
    /// percentage) the chance of being hit by fire during its own side's
    /// turn, i.e. reaction fire.
    pub jammer: i64,
    /// `HitShortRange`, `HitMediumRange`, `HitLongRange`: chance (percent,
    /// roughly) of a shot landing on the aimed tile in each range band.
    pub hit: [i64; 3],
    pub shot: ShotAnimation,
    /// `AIcategory`: row and column of the AI priority table.
    pub ai_category: i64,
    /// `isConvoy` and `isAITarget`: an AI-owned human unit whose two
    /// flags differ is a target for the AI (escorted convoys).
    pub is_convoy: bool,
    pub is_ai_target: bool,
    /// `Prerequisite`: building type a campaign player must own to deploy
    /// it.
    pub prerequisite: i64,
}

/// Which shot animations a unit type has (`Shoot*Animation_Begin` != -1)
/// and how its projectile flies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShotAnimation {
    /// `S`: raise the weapon.
    pub begin: bool,
    /// `T`: fire.
    pub shoot: bool,
    /// `U`: lower the weapon.
    pub end: bool,
    /// `P`: a projectile flies to the target.
    pub projectile: bool,
    /// `I`: impact at the target.
    pub impact: bool,
    /// `E`: the unit blows up.
    pub explosion: bool,
    /// `ShootShootAnimation_ProjBegin`: fire frame launching the projectile.
    pub proj_begin: i64,
    /// `ShootProjectileAnimation_PixOffset`: path pixels skipped at launch.
    pub pix_offset: i64,
    /// `ShootProjectileAnimation_PixSpeed`: path pixels per frame.
    pub pix_speed: i64,
    /// `ShootProjectileAnimation_Curve`: arc height in percent of the
    /// path length.
    pub curve: i64,
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurfaceType {
    pub id: i64,
    pub name: String,
    pub armor: i64,
    /// `ROADS.DBI` / `WALLS.DBI` prefix.
    pub image: String,
    /// Base editor: `canBeBuilt`, `Cost`, `Recycle` and (walls only)
    /// `prerequisite` (65535 none).
    pub can_build: bool,
    pub cost: i64,
    pub recycle: i64,
    pub prerequisite: i64,
}

/// Wreckage/debris type (`Ruin Types`); the id is its `RUINS.DBI` sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuinType {
    pub id: String,
    pub can_fire_through: bool,
    pub can_move_through: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MineType {
    pub id: i64,
    pub name: String,
    /// `UB.DBI` prefix.
    pub image: String,
    pub damage_base: i64,
    pub damage_random: i64,
    pub damage_type: i64,
    /// Base editor: `CanBuy`, `Cost`, `Recycle` and `prerequisite`.
    pub can_build: bool,
    pub cost: i64,
    pub recycle: i64,
    pub prerequisite: i64,
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
    /// `Tech Ratio`: `(max points, ratio)`, ascending; a negative maximum is
    /// unbounded.
    pub tech_ratios: Vec<(i64, i64)>,
    /// `Automated Units`: support units called in from off the map.
    pub automated: BTreeMap<i64, AutomatedType>,
    /// `Ranges`: distance bands for the hit chances.
    pub ranges: RangeBands,
    /// `AI Priority`: `[source][target]` AI category weights (missing
    /// cells are 0).
    pub ai_priority: [[i64; AI_CATEGORIES]; AI_CATEGORIES],
}

/// Rows and columns of the AI priority table (`AI Category Lookup`).
pub const AI_CATEGORIES: usize = 12;

/// Distance bands for [`UnitType::hit`]: up to `short` tiles is short
/// range, up to `medium` medium, farther long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeBands {
    pub short: i64,
    pub medium: i64,
}

impl Default for RangeBands {
    fn default() -> Self {
        Self {
            short: 3,
            medium: 6,
        }
    }
}

impl RangeBands {
    /// Band index (0 short, 1 medium, 2 long) for a distance in tiles.
    pub fn band(&self, distance: i64) -> usize {
        if distance <= self.short {
            0
        } else if distance <= self.medium {
            1
        } else {
            2
        }
    }
}

/// An automated (off-map support) unit type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomatedType {
    pub id: i64,
    pub name: String,
    /// `LISTBOX.DBI` portrait prefix; also the `AUTOUNIT\<image>0N.SMK`
    /// animation name.
    pub image: String,
    pub race: i64,
    /// `Automated Units Categories`: see [`AutomatedCategory`].
    pub category: i64,
    pub cost: i64,
    /// Campaign technology needed to buy it (`Prerequisite`).
    pub prerequisite: i64,
    /// Screen position of the anti-aircraft check in the animation.
    pub aa_pos: (i64, i64),
    /// Animation frame of the anti-aircraft check and of the effect.
    pub frame_aa: i64,
    pub frame_action: i64,
    pub cursor: String,
    /// Status bar prompt while choosing the target.
    pub explain: String,
    /// Usable once per battle.
    pub deploy_once: bool,
    /// `Automated Units Infos`, by `Automated Units Properties` number.
    pub infos: BTreeMap<i64, String>,
}

/// `Automated Units Categories`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomatedCategory {
    Destroy,
    Repair,
    Build,
    Mine,
    Drop,
    Transport,
}

/// `Automated Units Properties` numbers.
pub mod auto_prop {
    pub const ARMOR: i64 = 0;
    pub const DAMAGE: i64 = 1;
    pub const DAMAGE_TYPE: i64 = 2;
    pub const MINE_DETECTION_RANGE: i64 = 3;
    pub const DAMAGE_AREA: i64 = 4;
    pub const UNIT_ID_DROP: i64 = 5;
    pub const UNIT_TL_DROP: i64 = 6;
    pub const NB_DROP: i64 = 7;
    /// `unitPosX1`; X/Y pairs follow up to `unitPosY5` (17).
    pub const UNIT_POS: i64 = 8;
    pub const SECOND_ANIM_OK_BEGIN: i64 = 18;
    pub const SECOND_AA_POS_X: i64 = 24;
    pub const SECOND_AA_POS_Y: i64 = 25;
    pub const SECOND_FRAME_AA: i64 = 26;
    pub const SECOND_FRAME_ACTION: i64 = 27;
    pub const ARMOR_INC: i64 = 28;
    /// `facing1`; up to `facing5` (33).
    pub const FACING: i64 = 29;
    pub const SECOND_EXPLAIN: i64 = 34;
    pub const ACTION_INC: i64 = 35;
}

impl AutomatedType {
    pub fn category(&self) -> Option<AutomatedCategory> {
        use AutomatedCategory::*;
        Some(match self.category {
            0 => Destroy,
            1 => Repair,
            2 => Build,
            3 => Mine,
            4 => Drop,
            5 => Transport,
            _ => return None,
        })
    }

    /// Integer property, if set.
    pub fn int(&self, property: i64) -> Option<i64> {
        self.infos.get(&property)?.trim().parse().ok()
    }

    pub fn int_or(&self, property: i64, default: i64) -> i64 {
        self.int(property).unwrap_or(default)
    }

    /// Dropped units: offset from the target and facing, `nbDrop` of them.
    pub fn drops(&self) -> Vec<((i64, i64), i64)> {
        let n = self.int_or(auto_prop::NB_DROP, 0).clamp(0, 5);
        (0..n)
            .map(|i| {
                let x = self.int_or(auto_prop::UNIT_POS + 2 * i, 0);
                let y = self.int_or(auto_prop::UNIT_POS + 2 * i + 1, 0);
                ((x, y), self.int_or(auto_prop::FACING + i, 0))
            })
            .collect()
    }
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
                burned_min: t.float_or(r, "burned_min", 1.0)?.ceil() as i64,
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
                race: t.int_or(r, "Race", 0)?,
                destructible: t.flag_or(r, "isDestructible", true)?,
                has_ruin: t.flag_or(r, "hasRuin", false)?,
                explosion: match (
                    t.int_or(r, "ExplosionAnimation_Begin", -1)?,
                    t.int_or(r, "ExplosionAnimation_Qty", 0)?,
                ) {
                    (begin, qty) if (0..65535).contains(&begin) && qty > 0 => {
                        Some((begin as usize, qty as usize))
                    }
                    _ => None,
                },
                ai_category: t.int_or(r, "AIcategory", 0)?,
                cost: t.int_or(r, "Cost", 0)?,
                tech_reward: t.int_or(r, "TechReward", 0)?,
                can_build: t.flag_or(r, "CanBuild", false)?,
                prerequisite: t.int_or(r, "Prerequisite", 65535)?,
                recycle: t.int_or(r, "Recycle", 0)?,
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
                category: t.int_or(r, "category", 1)?,
                armor: t.int(r, "Armor")?,
                action_points: t.int(r, "actionPoints")?,
                shoot_action_points: t.int_or(r, "actionPointsShoot", 0)?,
                can_overwatch: t.flag_or(r, "canOverwatch", false)?,
                move_pix_speed: t.int_or(r, "MoveAnimation_PixSpeed", 8)?,
                move_animation: t.int_or(r, "MoveAnimation_Begin", -1)? != -1,
                move_start_animation: t.int_or(r, "MoveStartAnimation_Begin", -1)? != -1,
                move_end_animation: t.int_or(r, "MoveEndAnimation_Begin", -1)? != -1,
                image: t.text(r, "facingsImageID")?.to_ascii_uppercase(),
                cost: t.int_or(r, "Cost", 0)?,
                cost_to_next: t.int_or(r, "CostToNextTL", 0)?,
                damage_base: t.int_or(r, "damageBase", 0)?,
                damage_random: t.int_or(r, "damageRandom", 0)?,
                damage_type: t.int_or(r, "damageType", 0)?,
                damage_area: t.int_or(r, "DamageArea", 1)?,
                ruin: t.text_or(r, "ruinImageID")?.to_ascii_uppercase(),
                range: t.int_or(r, "weaponRange", 0)?,
                min_range: t.int_or(r, "WeaponMinimumRange", 0)?,
                has_weapon: t.flag_or(r, "UnitHasWeapon", true)?,
                anti_air: t.flag_or(r, "IsAA", false)?,
                damage_aa: t.int_or(r, "DamageAA", 0)?,
                mine_detection: t.int_or(r, "MineDetectionRange", 0)?,
                weapon_type: t.int_or(r, "weaponType", 0)?,
                jammer: t.int_or(r, "Jammer", 100)?,
                hit: [
                    t.int_or(r, "HitShortRange", 100)?,
                    t.int_or(r, "HitMediumRange", 100)?,
                    t.int_or(r, "HitLongRange", 100)?,
                ],
                shot: ShotAnimation {
                    begin: t.int_or(r, "ShootBeginAnimation_Begin", -1)? != -1,
                    shoot: t.int_or(r, "ShootShootAnimation_Begin", -1)? != -1,
                    end: t.int_or(r, "ShootEndAnimation_Begin", -1)? != -1,
                    projectile: t.int_or(r, "ShootProjectileAnimation_Begin", -1)? != -1,
                    impact: t.int_or(r, "ShootImpactAnimation_Begin", -1)? != -1,
                    explosion: t.int_or(r, "ExplosionAnimation_Begin", -1)? != -1,
                    proj_begin: t.int_or(r, "ShootShootAnimation_ProjBegin", 0)?,
                    pix_offset: t.int_or(r, "ShootProjectileAnimation_PixOffset", 0)?,
                    pix_speed: t.int_or(r, "ShootProjectileAnimation_PixSpeed", 20)?,
                    curve: t.int_or(r, "ShootProjectileAnimation_Curve", 0)?,
                },
                ai_category: t.int_or(r, "AIcategory", 0)?,
                is_convoy: t.flag_or(r, "isConvoy", false)?,
                is_ai_target: t.flag_or(r, "isAITarget", false)?,
                prerequisite: t.int_or(r, "Prerequisite", 65535)?,
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
                    can_build: t.flag_or(r, "canBeBuilt", false)?,
                    cost: t.int_or(r, "Cost", 0)?,
                    recycle: t.int_or(r, "Recycle", 0)?,
                    prerequisite: t.int_or(r, "prerequisite", 65535)?,
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
                damage_type: t.int_or(r, "damageType", 0)?,
                can_build: t.flag_or(r, "CanBuy", false)?,
                cost: t.int_or(r, "Cost", 0)?,
                recycle: t.int_or(r, "Recycle", 0)?,
                prerequisite: t.int_or(r, "prerequisite", 65535)?,
            };
            insert_unique(&mut defs.mines, &t.table, item.id, item)?;
        }

        // Optional; see `tech_points` for the default.
        if let Ok(t) = Records::read(global, "Tech Ratio") {
            for r in 0..t.len() {
                defs.tech_ratios
                    .push((t.int_or(r, "MaxRange", -1)?, t.int_or(r, "Ratio", 1)?));
            }
            defs.tech_ratios
                .sort_by_key(|&(max, _)| if max < 0 { i64::MAX } else { max });
        }
        if let Ok(t) = Records::read(global, "Ranges")
            && !t.is_empty()
        {
            defs.ranges = RangeBands {
                short: t.int_or(0, "Short", 3)?,
                medium: t.int_or(0, "Medium", 6)?,
            };
        }
        if let Ok(t) = Records::read(global, "AI Priority") {
            for r in 0..t.len() {
                let (s, c) = (t.int_or(r, "AiSource", -1)?, t.int_or(r, "AiTarget", -1)?);
                if let (Ok(s), Ok(c)) = (usize::try_from(s), usize::try_from(c))
                    && s < AI_CATEGORIES
                    && c < AI_CATEGORIES
                {
                    defs.ai_priority[s][c] = t.int_or(r, "Priority", 0)?;
                }
            }
        }
        if let Ok(t) = Records::read(global, "Automated Units") {
            for r in 0..t.len() {
                let item = AutomatedType {
                    id: t.int(r, "typeID")?,
                    name: t.text_or(r, "NameEnglish")?,
                    image: t.text_or(r, "facingsImageID")?.to_ascii_uppercase(),
                    race: t.int_or(r, "Race", 0)?,
                    category: t.int_or(r, "Category", 0)?,
                    cost: t.int_or(r, "Cost", 0)?,
                    prerequisite: t.int_or(r, "Prerequisite", -1)?,
                    aa_pos: (t.int_or(r, "AAPosX", 0)?, t.int_or(r, "AAPosY", 0)?),
                    frame_aa: t.int_or(r, "FrameAA", 0)?,
                    frame_action: t.int_or(r, "FrameAction", 0)?,
                    cursor: t.text_or(r, "cursorID")?.to_ascii_uppercase(),
                    explain: t.text_or(r, "ExplainEnglish")?,
                    deploy_once: t.flag_or(r, "DeployOnce", false)?,
                    infos: BTreeMap::new(),
                };
                insert_unique(&mut defs.automated, &t.table, item.id, item)?;
            }
        }
        if let Ok(t) = Records::read(global, "Automated Units Infos") {
            for r in 0..t.len() {
                let id = t.int(r, "typeID")?;
                let property = t.int(r, "Property")?;
                let value = t.text_or(r, "Value")?;
                if let Some(a) = defs.automated.get_mut(&id) {
                    a.infos.insert(property, value);
                }
            }
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

    /// Definition of a unit type at technology level `tech`, or at the
    /// highest defined level below it.
    pub fn unit_tech(&self, id: i64, tech: i64) -> Option<&UnitType> {
        self.units
            .range((id, 0)..=(id, tech.max(0)))
            .next_back()
            .map(|(_, u)| u)
            .or_else(|| self.unit(id))
    }

    /// Technology points of a skirmish worth `points` deployment points:
    /// `points` times the matching `Tech Ratio`.
    pub fn tech_points(&self, points: i64) -> i64 {
        const DEFAULT: [(i64, i64); 4] = [(300, 1), (700, 5), (1200, 8), (-1, 10)];
        let ratios = if self.tech_ratios.is_empty() {
            &DEFAULT[..]
        } else {
            &self.tech_ratios[..]
        };
        let ratio = ratios
            .iter()
            .find(|&&(max, _)| max < 0 || points <= max)
            .or(ratios.last())
            .map_or(1, |&(_, r)| r);
        points * ratio
    }
}
