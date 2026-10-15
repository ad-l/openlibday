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
    /// `negAP`: action points drained below 0 (player gas bomb, Tauran
    /// paralyzer), at most the unit's maximum; withheld from the next turn's
    /// restore. Always 0 in the stock scenarios and saves seen.
    pub neg_ap: i64,
    pub order: Order,
    /// Reinforcements only: `order`, an AI order derived from the objective.
    pub objective_order: bool,
    /// Overwatch requested (effective only for types that can overwatch).
    /// Not stored by the original game; on for every unit at load.
    pub overwatch: bool,
}

impl Unit {
    /// Removes `amount` action points; any shortfall below 0 becomes `negAP`,
    /// capped at `max` so a unit is never paralysed for more than one turn.
    pub fn drain_ap(&mut self, amount: i64, max: i64) {
        let left = self.action_points.unwrap_or(0) - amount.max(0);
        if left < 0 {
            self.neg_ap = (self.neg_ap - left).min(max.max(0));
        }
        self.action_points = Some(left.max(0));
    }

    /// Turn-start restore: full `max` action points less `negAP`, which is
    /// then cleared.
    pub fn restore_ap(&mut self, max: i64) {
        let max = max.max(0);
        self.action_points = Some(max - self.neg_ap.clamp(0, max));
        self.neg_ap = 0;
    }
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

impl Mine {
    /// Hidden from `player` (0 = the AI, 1-4 the humans), as `0x42c134`.
    pub fn hidden_to(&self, player: i64) -> bool {
        match player {
            0 => self.hidden_ai,
            p => self.hidden.get(p as usize - 1).copied().unwrap_or(true),
        }
    }

    /// Reveals the mine to `player` as `0x42c153` does: the AI alone, or
    /// every human player at once.
    pub fn reveal_to(&mut self, player: i64) {
        if player == 0 {
            self.hidden_ai = false;
        } else {
            self.hidden = [false; 4];
        }
    }
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

    /// The player owning an authored zone slot once merged for `humans`
    /// human players, as the original does on load (0x44049C): one human
    /// gets slots 1–4, two humans get 1+3 and 2+4, more keep their own.
    pub fn merged_player(&self, humans: usize) -> i64 {
        merged_owner(self.player, humans)
    }
}

/// Owner of an authored player slot once merged for `humans` human players:
/// one human takes slots 1–4, two humans take 1+3 and 2+4, more keep their
/// own. The alien (0) is never merged.
pub fn merged_owner(owner: i64, humans: usize) -> i64 {
    match (humans, owner) {
        (1, 1..=4) => 1,
        (2, 3 | 4) => owner - 2,
        _ => owner,
    }
}

/// Facing of a unit on `from` looking at tile `to` (the deployment rule's
/// buckets).
#[allow(clippy::approx_constant)]
pub fn facing_toward(from: (i64, i64), to: (i64, i64)) -> i64 {
    let rad = ((from.1 - to.1) as f64).atan2((from.0 - to.0) as f64);
    let mut deg = rad * 360.0 / 6.28318;
    if rad < 0.0 {
        deg += 360.0;
    }
    let e = (deg as i64 + 22) % 360;
    [2, 3, 4, 5, 6, 7, 0, 1]
        .get(e as usize / 45)
        .copied()
        .unwrap_or(0)
}

/// Facing of a unit placed on `(x, y)` of a `size`-tile map: toward the map
/// centre, as LIBDAY.EXE 0x43430b. The original takes `atan2` of the
/// centre-to-unit vector, converts it to whole degrees with its 6.28318
/// approximation of 2π, offsets by 22° and buckets by 45°.
pub fn deploy_facing(size: i64, x: i64, y: i64) -> i64 {
    let c = size / 2;
    facing_toward((x, y), (c, c))
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
    /// `TechPoints`: an openlibday-only override of the technology points
    /// derived from `points`. The original ignores the column; 0 skips the
    /// technology screen.
    pub tech_points: Option<i64>,
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
    /// Reassigns the authored deployment slots to `humans` human players
    /// (see [`Deployment::merged_player`]), dropping duplicate tiles.
    pub fn merge_deployments(&mut self, humans: usize) {
        let mut seen = std::collections::HashSet::new();
        self.deployments.retain_mut(|d| {
            d.player = d.merged_player(humans);
            seen.insert((d.player, d.turn, d.x, d.y))
        });
    }

    /// Merges player slots for `humans` human players at load: deployment
    /// zones, plus the owners of units, reinforcements, buildings, roads,
    /// walls and mines (so with one human, player 2's units become player 1's).
    pub fn merge_players(&mut self, humans: usize) {
        self.merge_deployments(humans);
        let owners = self
            .units
            .iter_mut()
            .map(|u| &mut u.owner)
            .chain(self.buildings.iter_mut().map(|b| &mut b.owner))
            .chain(self.roads.iter_mut().map(|s| &mut s.owner))
            .chain(self.walls.iter_mut().map(|s| &mut s.owner))
            .chain(self.mines.iter_mut().map(|m| &mut m.owner));
        for owner in owners {
            *owner = merged_owner(*owner, humans);
        }
    }

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
                tech_points: Some(t.int_or(0, "TechPoints", -1)?).filter(|&p| p >= 0),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployed_units_face_the_map_centre() {
        // Facing 0 = +y (south), 2 = -x, 4 = -y, 6 = +x.
        assert_eq!(deploy_facing(64, 60, 32), 2);
        assert_eq!(deploy_facing(64, 4, 32), 6);
        assert_eq!(deploy_facing(64, 32, 60), 4);
        assert_eq!(deploy_facing(64, 32, 4), 0);
        assert_eq!(deploy_facing(64, 4, 60), 5);
        assert_eq!(deploy_facing(64, 60, 4), 1);
        assert_eq!(deploy_facing(64, 4, 4), 7);
        assert_eq!(deploy_facing(64, 60, 60), 3);
        assert_eq!(deploy_facing(64, 32, 32), 2);
    }

    #[test]
    fn deployment_slots_merge_by_human_count() {
        let d = |player| Deployment {
            counter: 0,
            player,
            x: 0,
            y: 0,
            turn: 0,
        };
        let owners = |humans| {
            (0..=4)
                .map(|p| d(p).merged_player(humans))
                .collect::<Vec<_>>()
        };
        assert_eq!(owners(1), [0, 1, 1, 1, 1]);
        assert_eq!(owners(2), [0, 1, 2, 1, 2]);
        assert_eq!(owners(3), [0, 1, 2, 3, 4]);
        assert_eq!(owners(4), [0, 1, 2, 3, 4]);
    }

    fn unit(ap: i64) -> Unit {
        Unit {
            counter: 1,
            id: 1,
            kind: 1,
            tech: 0,
            owner: 1,
            facing: 0,
            x: 0,
            y: 0,
            arrival: 0,
            reinforcement: false,
            armor: None,
            action_points: Some(ap),
            neg_ap: 0,
            order: Order::default(),
            objective_order: false,
            overwatch: true,
        }
    }

    #[test]
    fn negative_ap_carries_into_next_turn_once() {
        let mut u = unit(40);
        u.drain_ap(100, 250);
        assert_eq!((u.action_points, u.neg_ap), (Some(0), 60));
        u.restore_ap(250);
        assert_eq!((u.action_points, u.neg_ap), (Some(190), 0));
        u.restore_ap(250);
        assert_eq!(u.action_points, Some(250));
    }

    #[test]
    fn negative_ap_is_capped_at_maximum() {
        let mut u = unit(10);
        u.drain_ap(200, 120);
        u.drain_ap(200, 120);
        assert_eq!(u.neg_ap, 120);
        u.restore_ap(120);
        assert_eq!(u.action_points, Some(0));
        u.restore_ap(120);
        assert_eq!(u.action_points, Some(120));
    }

    #[test]
    fn owners_merge_by_human_count() {
        let merged = |humans| (0..=4).map(|o| merged_owner(o, humans)).collect::<Vec<_>>();
        assert_eq!(merged(1), [0, 1, 1, 1, 1]);
        assert_eq!(merged(2), [0, 1, 2, 1, 2]);
        assert_eq!(merged(3), [0, 1, 2, 3, 4]);
    }
}
