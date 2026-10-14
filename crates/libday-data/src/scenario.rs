//! A tactical scenario database (`SCENS\*.ZIP` member or loose `.mdb`).
//!
//! Layers are kept in the order the original tables are queried by the
//! editor (`ORDER BY` id/counter), which is also their draw order.

use libday_formats::Database;
use serde::{Deserialize, Serialize};

use crate::DataError;
use crate::records::Records;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub id: i64,
    /// `TERRAINS.DBI` artwork identifier (upper case).
    pub terrain: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Building {
    pub counter: i64,
    pub id: i64,
    pub kind: i64,
    pub owner: i64,
    pub armor: i64,
    pub x: i64,
    pub y: i64,
    pub ai_target: bool,
}

/// AI order of a unit on the map (`OrderID`, `Turn`, `Probability`,
/// `EnemyUnitType`). Targets are in [`Scenario::order_targets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    /// `Unit Order Id Lookup`: 0 none, 1 goto, 2 goto fast, 3 escort,
    /// 4 defend, 5 patrol, 6 attack.
    pub id: i64,
    /// Inclusive expiry turn: cleared once the current turn is later.
    pub expiry: i64,
    /// Retention roll: cleared when `rand() % 100` exceeds it.
    pub probability: i64,
    pub enemy_type: i64,
}

impl Default for Order {
    /// The editor's defaults for a new unit.
    fn default() -> Self {
        Self {
            id: 0,
            expiry: 0,
            probability: 100,
            enemy_type: 65535,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    pub counter: i64,
    pub id: i64,
    pub kind: i64,
    pub tech: i64,
    pub owner: i64,
    pub facing: i64,
    pub x: i64,
    pub y: i64,
    /// 0 for units on the map at the start; the arrival turn of reinforcements.
    pub arrival: i64,
    /// Row from `Reinforcement` rather than `Units On Map`.
    pub reinforcement: bool,
    pub armor: Option<i64>,
    pub action_points: Option<i64>,
    /// `negAP` (always 0 in the stock scenarios and saves seen).
    pub neg_ap: i64,
    pub order: Order,
    /// Reinforcements only: `order`, an AI order derived from the objective.
    pub objective_order: bool,
    /// Overwatch requested (effective only for types that can overwatch).
    /// Not stored by the original game; on for every unit at load.
    pub overwatch: bool,
}

/// A target of a unit's AI order: another unit or building
/// (`Unit Order ID List`) or a position (`Unit Order Pos List`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderTarget {
    pub counter: i64,
    pub unit: i64,
    #[serde(flatten)]
    pub target: Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Id(i64),
    Position { x: i64, y: i64 },
}

/// A road (`Roads On Map`) or wall (`Walls On Map`) tile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Surface {
    pub counter: i64,
    pub kind: i64,
    pub owner: i64,
    pub x: i64,
    pub y: i64,
    pub armor: i64,
    /// Walls only: `isDamaged`.
    pub broken: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruin {
    pub counter: i64,
    /// `RUINS.DBI` sprite / `Ruin Types.id` (upper case).
    pub id: String,
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mine {
    pub counter: i64,
    pub kind: i64,
    pub owner: i64,
    pub x: i64,
    pub y: i64,
    pub armed: bool,
    pub hidden_ai: bool,
    /// Hidden from players 1..4.
    pub hidden: [bool; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deployment {
    pub counter: i64,
    pub player: i64,
    pub x: i64,
    pub y: i64,
    pub turn: i64,
}

impl Deployment {
    /// Human-player starting zone (the only kind the deployment screen shows).
    pub fn is_start_zone(&self) -> bool {
        (1..=4).contains(&self.player) && self.turn == 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenarioInfo {
    pub title: String,
    /// Region name shown under the title in the goal dialog.
    pub province: String,
    pub description: String,
    pub turns: i64,
    /// `One Off Deployment`: technology/deployment points of a skirmish.
    pub points: i64,
    /// `NextID_Units` / `NextID_Buildings`. The original does not update
    /// them in savegames; see [`Scenario::next_unit_id`].
    pub next_unit_id: i64,
    pub next_building_id: i64,
    /// `ReinfRes`: credits a player receives each turn.
    pub reinf_res: i64,
    /// Unit types a skirmish player may buy and upgrade (`One Off Units
    /// Available`, in table order).
    pub available_units: Vec<i64>,
    /// Automated (building) types shown on the technology screen (`One Off
    /// Automated Available`).
    pub available_automated: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenario {
    pub size: usize,
    /// Row-major (`y * size + x`).
    pub cells: Vec<Cell>,
    pub buildings: Vec<Building>,
    /// Units on the map, then reinforcements.
    pub units: Vec<Unit>,
    pub roads: Vec<Surface>,
    pub walls: Vec<Surface>,
    pub ruins: Vec<Ruin>,
    pub mines: Vec<Mine>,
    pub deployments: Vec<Deployment>,
    /// AI order targets: `Unit Order ID List`, then `Unit Order Pos List`.
    pub order_targets: Vec<OrderTarget>,
    pub info: ScenarioInfo,
}

fn invalid(table: &str, message: impl Into<String>) -> DataError {
    DataError::Invalid {
        table: table.to_owned(),
        message: message.into(),
    }
}

impl Scenario {
    pub fn load(db: &Database) -> Result<Self, DataError> {
        let t = Records::read(db, "Map")?;
        let size = t.len().isqrt();
        if t.is_empty() || size * size != t.len() || size > 256 {
            return Err(invalid(
                "Map",
                "the map must be a complete square, at most 256x256",
            ));
        }
        let mut cells: Vec<Option<Cell>> = vec![None; t.len()];
        for r in 0..t.len() {
            let (x, y) = (t.int(r, "PosX")?, t.int(r, "PosY")?);
            if !(0..size as i64).contains(&x) || !(0..size as i64).contains(&y) {
                return Err(invalid("Map", format!("cell {x},{y} is outside the map")));
            }
            let slot = &mut cells[y as usize * size + x as usize];
            if slot.is_some() {
                return Err(invalid("Map", format!("duplicate cell {x},{y}")));
            }
            *slot = Some(Cell {
                id: t.int(r, "Id")?,
                terrain: t.text(r, "TerrainImg")?.to_ascii_uppercase(),
            });
        }
        let cells = cells.into_iter().map(Option::unwrap).collect();

        let t = Records::read(db, "Buildings On Map")?;
        let mut buildings = Vec::with_capacity(t.len());
        for r in 0..t.len() {
            buildings.push(Building {
                counter: t.int(r, "Counter")?,
                id: t.int(r, "id")?,
                kind: t.int(r, "buildingType")?,
                owner: t.int(r, "playerOwner")?,
                armor: t.int(r, "currentAR")?,
                x: t.int(r, "PosX")?,
                y: t.int(r, "PosY")?,
                ai_target: t.flag(r, "IsAiTarget")?,
            });
        }
        buildings.sort_by_key(|b| b.id);

        let t = Records::read(db, "Units On Map")?;
        let mut units = Vec::with_capacity(t.len());
        for r in 0..t.len() {
            units.push(Unit {
                counter: t.int(r, "Counter")?,
                id: t.int(r, "ID")?,
                kind: t.int(r, "unitType")?,
                tech: t.int(r, "unitTL")?,
                owner: t.int(r, "playerOwner")?,
                facing: t.int(r, "facing")?,
                x: t.int(r, "PosX")?,
                y: t.int(r, "PosY")?,
                arrival: 0,
                reinforcement: false,
                armor: Some(t.int(r, "currentAR")?),
                action_points: Some(t.int(r, "currentAP")?),
                neg_ap: t.int_or(r, "negAP", 0)?,
                order: Order {
                    id: t.int_or(r, "OrderID", 0)?,
                    expiry: t.int_or(r, "Turn", 0)?,
                    probability: t.int_or(r, "Probability", 100)?,
                    enemy_type: t.int_or(r, "EnemyUnitType", 65535)?,
                },
                objective_order: false,
                overwatch: true,
            });
        }
        units.sort_by_key(|u| u.id);
        let t = Records::read(db, "Reinforcement")?;
        let mut reinforcements = Vec::with_capacity(t.len());
        for r in 0..t.len() {
            reinforcements.push(Unit {
                counter: t.int(r, "Counter")?,
                id: t.int(r, "UnitID")?,
                kind: t.int(r, "type")?,
                tech: 0,
                owner: 0,
                facing: 0,
                x: t.int(r, "posX")?,
                y: t.int(r, "posY")?,
                arrival: t.int(r, "turn")?,
                reinforcement: true,
                armor: None,
                action_points: None,
                neg_ap: 0,
                order: Order::default(),
                objective_order: t.flag_or(r, "order", false)?,
                overwatch: true,
            });
        }
        reinforcements.sort_by_key(|u| u.id);
        units.extend(reinforcements);

        let surfaces = |table: &str, wall: bool| -> Result<Vec<Surface>, DataError> {
            let t = Records::read(db, table)?;
            let mut out = Vec::with_capacity(t.len());
            for r in 0..t.len() {
                out.push(Surface {
                    counter: t.int(r, "Counter")?,
                    kind: t.int(r, "roadtype")?,
                    owner: t.int_or(r, "playerOwner", 0)?,
                    x: t.int(r, "PosX")?,
                    y: t.int(r, "PosY")?,
                    armor: t.int_or(r, "currentAR", 0)?,
                    broken: wall && t.flag_or(r, "isDamaged", false)?,
                });
            }
            out.sort_by_key(|s| s.counter);
            Ok(out)
        };
        let roads = surfaces("Roads On Map", false)?;
        let walls = surfaces("Walls On Map", true)?;

        let t = Records::read(db, "Ruins On Map")?;
        let mut ruins = Vec::with_capacity(t.len());
        for r in 0..t.len() {
            ruins.push(Ruin {
                counter: t.int(r, "Counter")?,
                id: t.text(r, "id")?.to_ascii_uppercase(),
                x: t.int(r, "PosX")?,
                y: t.int(r, "PosY")?,
            });
        }
        ruins.sort_by_key(|s| s.counter);

        let t = Records::read(db, "Mines On Map")?;
        let mut mines = Vec::with_capacity(t.len());
        for r in 0..t.len() {
            mines.push(Mine {
                counter: t.int(r, "Counter")?,
                kind: t.int(r, "typeID")?,
                owner: t.int_or(r, "playerOwner", 0)?,
                x: t.int(r, "PosX")?,
                y: t.int(r, "PosY")?,
                armed: t.flag(r, "armed")?,
                hidden_ai: t.flag(r, "hidden_AI")?,
                hidden: [
                    t.flag(r, "hidden_1")?,
                    t.flag(r, "hidden_2")?,
                    t.flag(r, "hidden_3")?,
                    t.flag(r, "hidden_4")?,
                ],
            });
        }
        mines.sort_by_key(|s| s.counter);

        let t = Records::read(db, "Deployment")?;
        let mut deployments = Vec::with_capacity(t.len());
        for r in 0..t.len() {
            deployments.push(Deployment {
                counter: t.int(r, "Counter")?,
                player: t.int(r, "player")?,
                x: t.int(r, "PosX")?,
                y: t.int(r, "PosY")?,
                turn: t.int_or(r, "Turn", 0)?,
            });
        }
        deployments.sort_by_key(|d| d.counter);

        let mut order_targets = Vec::new();
        let t = Records::read(db, "Unit Order ID List")?;
        for r in 0..t.len() {
            order_targets.push(OrderTarget {
                counter: t.int(r, "Counter")?,
                unit: t.int(r, "Unit ID")?,
                target: Target::Id(t.int(r, "Id")?),
            });
        }
        let t = Records::read(db, "Unit Order Pos List")?;
        for r in 0..t.len() {
            order_targets.push(OrderTarget {
                counter: t.int(r, "Counter")?,
                unit: t.int(r, "Unit ID")?,
                target: Target::Position {
                    x: t.int(r, "PosX")?,
                    y: t.int(r, "PosY")?,
                },
            });
        }
        order_targets.sort_by_key(|o| (matches!(o.target, Target::Position { .. }), o.counter));

        let t = Records::read(db, "Scenario Info")?;
        let info = if t.is_empty() {
            ScenarioInfo::default()
        } else {
            ScenarioInfo {
                title: t.text_or(0, "TitleEnglish")?,
                province: t.text_or(0, "provincenameEnglish")?,
                description: t.text_or(0, "descriptionEnglish")?,
                turns: t.int_or(0, "turn_qty", 0)?,
                points: t.int_or(0, "One Off Deployment", 0)?,
                next_unit_id: t.int_or(0, "NextID_Units", 0)?,
                next_building_id: t.int_or(0, "NextID_Buildings", 0)?,
                reinf_res: t.int_or(0, "ReinfRes", 0)?,
                available_units: available(db, "One Off Units Available"),
                available_automated: available(db, "One Off Automated Available"),
            }
        };

        Ok(Self {
            size,
            cells,
            buildings,
            units,
            roads,
            walls,
            ruins,
            mines,
            deployments,
            order_targets,
            info,
        })
    }

    /// ID for a new unit: `NextID_Units`, or past the largest ID in use (the
    /// original numbers deployed units that way without updating the field).
    pub fn next_unit_id(&self) -> i64 {
        let used = self.units.iter().map(|u| u.id + 1).max().unwrap_or(0);
        used.max(self.info.next_unit_id)
    }

    pub fn contains(&self, x: i64, y: i64) -> bool {
        (0..self.size as i64).contains(&x) && (0..self.size as i64).contains(&y)
    }

    pub fn cell(&self, x: i64, y: i64) -> Option<&Cell> {
        self.contains(x, y)
            .then(|| &self.cells[y as usize * self.size + x as usize])
    }
}

/// `Enum` of the `Available` rows of a `One Off * Available` table, in
/// `Counter` order; empty when the table is missing or malformed.
fn available(db: &Database, table: &str) -> Vec<i64> {
    let Ok(t) = Records::read(db, table) else {
        return Vec::new();
    };
    let mut rows: Vec<(i64, i64)> = (0..t.len())
        .filter(|&r| t.flag_or(r, "Available", false).unwrap_or(false))
        .filter_map(|r| Some((t.int(r, "Counter").ok()?, t.int(r, "Enum").ok()?)))
        .collect();
    rows.sort_unstable();
    rows.into_iter().map(|(_, e)| e).collect()
}

/// An entry of the skirmish selector catalog (`SCENS\SKIRMISH.MDB`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkirmishEntry {
    pub counter: i64,
    pub title: String,
    pub description: String,
    pub points: i64,
    pub size: i64,
    /// Scenario database name inside the scenario archives.
    pub file_name: String,
}

pub fn skirmish_catalog(db: &Database) -> Result<Vec<SkirmishEntry>, DataError> {
    let t = Records::read(db, "Scenario Info")?;
    let mut out = Vec::with_capacity(t.len());
    for r in 0..t.len() {
        out.push(SkirmishEntry {
            counter: t.int(r, "Counter")?,
            title: t.text(r, "TitleEnglish")?,
            description: t.text_or(r, "descriptionEnglish")?,
            points: t.int_or(r, "One Off Deployment", 0)?,
            size: t.int_or(r, "Size", 0)?,
            file_name: t.text(r, "FileName")?,
        });
    }
    out.sort_by_key(|e| e.counter);
    Ok(out)
}

/// Difficulty shown in the skirmish selector for `points` deployment points.
pub fn difficulty_label(points: i64) -> &'static str {
    match points {
        ..=300 => "Easy",
        301..=500 => "Average",
        501..=1000 => "Hard",
        _ => "Very hard",
    }
}
