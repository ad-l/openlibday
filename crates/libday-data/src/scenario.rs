//! A tactical scenario database (`SCENS\*.ZIP` member or loose `.mdb`).
//!
//! Layers are kept in the order the original tables are queried by the
//! editor (`ORDER BY` id/counter), which is also their draw order.

use libday_formats::Database;

use crate::DataError;
use crate::records::Records;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub id: i64,
    /// `TERRAINS.DBI` artwork identifier (upper case).
    pub terrain: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
}

/// A road (`Roads On Map`) or wall (`Walls On Map`) tile.
#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ruin {
    pub counter: i64,
    /// `RUINS.DBI` sprite / `Ruin Types.id` (upper case).
    pub id: String,
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub description: String,
    pub turns: i64,
    /// `One Off Deployment`: technology/deployment points of a skirmish.
    pub points: i64,
}

#[derive(Clone, Debug)]
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

        let t = Records::read(db, "Scenario Info")?;
        let info = if t.is_empty() {
            ScenarioInfo::default()
        } else {
            ScenarioInfo {
                title: t.text_or(0, "TitleEnglish")?,
                description: t.text_or(0, "descriptionEnglish")?,
                turns: t.int_or(0, "turn_qty", 0)?,
                points: t.int_or(0, "One Off Deployment", 0)?,
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
            info,
        })
    }

    pub fn contains(&self, x: i64, y: i64) -> bool {
        (0..self.size as i64).contains(&x) && (0..self.size as i64).contains(&y)
    }

    pub fn cell(&self, x: i64, y: i64) -> Option<&Cell> {
        self.contains(x, y)
            .then(|| &self.cells[y as usize * self.size + x as usize])
    }
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
