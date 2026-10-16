//! Battle state and savegames.
//!
//! A [`Battle`] is a scenario in play: the scenario layers as they are now
//! (moved, damaged, destroyed and arrived units, burnt terrain, ...) plus the
//! turn state the original keeps in `GAMEINFO.MDB`.
//!
//! openlibday writes its own versioned JSON format. It names the source
//! scenario, which still provides the static data (deployment zones, goals,
//! texts), and stores every dynamic layer in full plus the terrain cells that
//! differ from the source. Original savegames (`SAVEGAME\savegame.NN\`) can be
//! imported; see [`Battle::import`].

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use libday_formats::{Database, Zip};
use serde::{Deserialize, Serialize};

use crate::records::Records;
use crate::scenario::{Building, Mine, OrderTarget, Ruin, Scenario, Surface, Unit};
use crate::{DataError, Dir, GameData, SCENARIO_ARCHIVES, find_case_insensitive};

/// `format` field of an openlibday battle save.
pub const FORMAT: &str = "openlibday-battle";
/// Current save version; older versions are migrated on load.
pub const VERSION: u32 = 1;

fn error(message: impl Into<String>) -> DataError {
    DataError::Savegame(message.into())
}

/// Where the static scenario data and the original terrain come from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A scenario database name, looked up like [`GameData::scenario_bytes`].
    Scenario(String),
    /// A scenario database file, e.g. an original savegame's `SCENARIO.MDB`
    /// that could not be matched to a shipped scenario.
    File(PathBuf),
}

impl Source {
    pub fn load(&self, game: &GameData) -> Result<Scenario, DataError> {
        match self {
            Source::Scenario(name) => game.load_scenario(name),
            Source::File(path) => load_file(path),
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Scenario(name) => f.write_str(name),
            Source::File(path) => write!(f, "{}", path.display()),
        }
    }
}

/// Loads a scenario database file.
pub fn load_file(path: &Path) -> Result<Scenario, DataError> {
    let name = path.display().to_string();
    let bytes = fs::read(path).map_err(|source| DataError::Io {
        path: path.to_owned(),
        source,
    })?;
    let db = Database::open(bytes).map_err(|e| DataError::Database(name, e))?;
    Scenario::load(&db)
}

/// Turn state (`GAMEINFO.MDB` `Tactic IO`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    /// `Current Turn`, from 1.
    pub turn: i64,
    /// `Current Player`: the player slot whose turn it is.
    pub player: i64,
    /// `Objective Turn` and `Last Turn` (0 and -1 in every save seen).
    pub objective_turn: i64,
    pub last_turn: i64,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            turn: 1,
            player: 1,
            objective_turn: 0,
            last_turn: -1,
        }
    }
}

/// Tactical statistics of one player (`Player Tactical*` tables); the
/// counts are keyed by unit or building type.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerRecord {
    pub player: i64,
    /// `Deploy Res`: deployment points left.
    pub deploy_points: i64,
    pub units_destroyed: BTreeMap<i64, i64>,
    pub units_lost: BTreeMap<i64, i64>,
    pub buildings_destroyed: BTreeMap<i64, i64>,
    pub buildings_lost: BTreeMap<i64, i64>,
}

impl PlayerRecord {
    /// The battle report's rank (`0x415746`): twice the cost of the units
    /// destroyed less that of the units lost, plus the cost of the
    /// buildings destroyed less that of the buildings lost. Unit costs are
    /// at technology level 0, as in the original.
    pub fn rank(&self, defs: &crate::Definitions) -> i64 {
        let units: i64 = defs
            .units
            .iter()
            .filter(|((_, tech), _)| *tech == 0)
            .map(|(&(id, _), k)| {
                let d = self.units_destroyed.get(&id).copied().unwrap_or(0);
                let l = self.units_lost.get(&id).copied().unwrap_or(0);
                (2 * d - l) * k.cost
            })
            .sum();
        let buildings: i64 = defs
            .buildings
            .iter()
            .map(|(id, k)| {
                let d = self.buildings_destroyed.get(id).copied().unwrap_or(0);
                let l = self.buildings_lost.get(id).copied().unwrap_or(0);
                (d - l) * k.cost
            })
            .sum();
        units + buildings
    }

    /// Total units destroyed, units lost, buildings destroyed and lost.
    pub fn totals(&self) -> [i64; 4] {
        let sum = |m: &BTreeMap<i64, i64>| m.values().sum();
        [
            sum(&self.units_destroyed),
            sum(&self.units_lost),
            sum(&self.buildings_destroyed),
            sum(&self.buildings_lost),
        ]
    }
}

/// A scenario in play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Battle {
    /// Save name (the original's `.dsc` file name).
    pub name: String,
    pub source: Source,
    /// Current state of every scenario layer.
    pub scenario: Scenario,
    pub clock: Clock,
    /// `Server Mode Info`: `Difficulty` and `Turn Time` (the turn timer
    /// setting; the elapsed time is not saved by the original).
    pub difficulty: i64,
    pub turn_time: i64,
    pub auto_fire: bool,
    pub players: Vec<PlayerRecord>,
}

impl Battle {
    /// A battle starting on turn 1.
    pub fn new(source: Source, scenario: Scenario) -> Self {
        Self {
            name: scenario.info.title.clone(),
            source,
            scenario,
            clock: Clock::default(),
            difficulty: 10,
            turn_time: 1,
            auto_fire: true,
            players: Vec::new(),
        }
    }

    /// Starts the shipped scenario `name` (e.g. `s1-3-1.mdb`).
    pub fn start(game: &GameData, name: &str) -> Result<Self, DataError> {
        let source = Source::Scenario(name.to_owned());
        let scenario = source.load(game)?;
        Ok(Self::new(source, scenario))
    }

    /// Player `player`'s record, created on first use.
    pub fn record_mut(&mut self, player: i64) -> &mut PlayerRecord {
        let i = match self.players.iter().position(|p| p.player == player) {
            Some(i) => i,
            None => {
                self.players.push(PlayerRecord {
                    player,
                    ..PlayerRecord::default()
                });
                self.players.len() - 1
            }
        };
        &mut self.players[i]
    }

    /// Counts what an attack by player `attacker` destroyed: the units
    /// on the map and the buildings of `before` missing from the current
    /// scenario are lost to their owners and, unless they were the
    /// attacker's own, destroyed by the attacker.
    pub fn tally(&mut self, before: &Scenario, attacker: i64) {
        let units: Vec<(i64, i64)> = before
            .units
            .iter()
            .filter(|u| u.arrival <= 0)
            .filter(|u| {
                // A unit lifted by a transport (arrival -1) is not lost.
                !self
                    .scenario
                    .units
                    .iter()
                    .any(|n| n.id == u.id && n.arrival <= 0)
            })
            .map(|u| (u.kind, u.owner))
            .collect();
        let buildings: Vec<(i64, i64)> = before
            .buildings
            .iter()
            .filter(|b| !self.scenario.buildings.iter().any(|n| n.id == b.id))
            .map(|b| (b.kind, b.owner))
            .collect();
        for (kind, owner) in units {
            *self.record_mut(owner).units_lost.entry(kind).or_default() += 1;
            if crate::objective::hostile(owner, attacker) {
                *self
                    .record_mut(attacker)
                    .units_destroyed
                    .entry(kind)
                    .or_default() += 1;
            }
        }
        for (kind, owner) in buildings {
            *self
                .record_mut(owner)
                .buildings_lost
                .entry(kind)
                .or_default() += 1;
            if crate::objective::hostile(owner, attacker) {
                *self
                    .record_mut(attacker)
                    .buildings_destroyed
                    .entry(kind)
                    .or_default() += 1;
            }
        }
    }

    /// Serializes against `base`, the scenario loaded from [`Battle::source`].
    pub fn to_json(&self, base: &Scenario) -> Result<String, DataError> {
        let s = &self.scenario;
        if base.size != s.size {
            return Err(error(format!(
                "{} is {}x{} but the battle map is {}x{}",
                self.source, base.size, base.size, s.size, s.size
            )));
        }
        let terrain = s
            .cells
            .iter()
            .zip(&base.cells)
            .enumerate()
            .filter(|(_, (now, then))| now.terrain != then.terrain)
            .map(|(i, (now, _))| TerrainChange {
                x: (i % s.size) as i64,
                y: (i / s.size) as i64,
                terrain: now.terrain.clone(),
            })
            .collect();
        let file = SaveFile {
            format: FORMAT.to_owned(),
            version: VERSION,
            name: self.name.clone(),
            source: self.source.clone(),
            size: s.size,
            clock: self.clock,
            difficulty: self.difficulty,
            turn_time: self.turn_time,
            auto_fire: self.auto_fire,
            players: self.players.clone(),
            terrain,
            units: s.units.clone(),
            buildings: s.buildings.clone(),
            walls: s.walls.clone(),
            roads: s.roads.clone(),
            ruins: s.ruins.clone(),
            mines: s.mines.clone(),
            order_targets: s.order_targets.clone(),
        };
        serde_json::to_string_pretty(&file).map_err(|e| error(e.to_string()))
    }

    /// Restores a battle from [`Battle::to_json`] output and its `base`
    /// scenario (see [`Battle::source_of`]).
    pub fn from_json(text: &str, base: Scenario) -> Result<Self, DataError> {
        let file = parse(text)?;
        let mut scenario = base;
        if scenario.size != file.size {
            return Err(error(format!(
                "{} is {}x{} but the save is {}x{}",
                file.source, scenario.size, scenario.size, file.size, file.size
            )));
        }
        for change in file.terrain {
            if !scenario.contains(change.x, change.y) {
                return Err(error(format!(
                    "terrain change {},{} is outside the map",
                    change.x, change.y
                )));
            }
            let i = change.y as usize * scenario.size + change.x as usize;
            scenario.cells[i].terrain = change.terrain.to_ascii_uppercase();
        }
        scenario.units = file.units;
        scenario.buildings = file.buildings;
        scenario.walls = file.walls;
        scenario.roads = file.roads;
        scenario.ruins = file.ruins;
        scenario.mines = file.mines;
        scenario.order_targets = file.order_targets;
        Ok(Self {
            name: file.name,
            source: file.source,
            scenario,
            clock: file.clock,
            difficulty: file.difficulty,
            turn_time: file.turn_time,
            auto_fire: file.auto_fire,
            players: file.players,
        })
    }

    /// The source named by a save, without restoring it.
    pub fn source_of(text: &str) -> Result<Source, DataError> {
        Ok(parse(text)?.source)
    }

    /// Writes the battle to `path`, creating its directory.
    pub fn save(&self, game: &GameData, path: &Path) -> Result<(), DataError> {
        let json = self.to_json(&self.source.load(game)?)?;
        let io = |source| DataError::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io)?;
        }
        // Write then rename so a failed write keeps the previous save.
        let temp = path.with_extension("tmp");
        fs::write(&temp, json).map_err(io)?;
        fs::rename(&temp, path).map_err(io)
    }

    /// Loads an openlibday save file or an original savegame directory.
    pub fn open(game: &GameData, path: &Path) -> Result<Self, DataError> {
        if path.is_dir() {
            return Self::import(game, path);
        }
        let text = fs::read_to_string(path).map_err(|source| DataError::Io {
            path: path.to_owned(),
            source,
        })?;
        let base = Self::source_of(&text)?.load(game)?;
        Self::from_json(&text, base)
    }

    /// Imports an original battle savegame (`SAVEGAME\savegame.NN\`).
    ///
    /// Its `SCENARIO.MDB` is a full scenario database holding the current
    /// state and `GAMEINFO.MDB` the turn state. Saves made outside a battle
    /// have no `SCENARIO.MDB` and are rejected. The source is the shipped
    /// scenario with the same title and the fewest terrain differences, or
    /// the savegame's own database when none matches.
    pub fn import(game: &GameData, dir: &Path) -> Result<Self, DataError> {
        let path = find_case_insensitive(dir, "SCENARIO.MDB").ok_or_else(|| {
            error(format!(
                "{} has no SCENARIO.MDB (it was not saved during a battle)",
                dir.display()
            ))
        })?;
        let scenario = load_file(&path)?;
        let source = identify(game, &scenario)?
            .map(Source::Scenario)
            .unwrap_or(Source::File(path));
        let mut battle = Self::new(source, scenario);
        if let Some(name) = description(dir) {
            battle.name = name;
        }
        if let Some(path) = find_case_insensitive(dir, "GAMEINFO.MDB") {
            let bytes = fs::read(&path).map_err(|source| DataError::Io {
                path: path.clone(),
                source,
            })?;
            let db = Database::open(bytes)
                .map_err(|e| DataError::Database(path.display().to_string(), e))?;
            battle.read_game_info(&db)?;
        }
        Ok(battle)
    }

    fn read_game_info(&mut self, db: &Database) -> Result<(), DataError> {
        let t = Records::read(db, "Tactic IO")?;
        if !t.is_empty() {
            self.clock = Clock {
                turn: t.int_or(0, "Current Turn", 1)?,
                player: t.int_or(0, "Current Player", 1)?,
                objective_turn: t.int_or(0, "Objective Turn", 0)?,
                last_turn: t.int_or(0, "Last Turn", -1)?,
            };
        }
        let t = Records::read(db, "Server Mode Info")?;
        if !t.is_empty() {
            self.difficulty = t.int_or(0, "Difficulty", self.difficulty)?;
            self.turn_time = t.int_or(0, "Turn Time", self.turn_time)?;
        }
        let mut players: BTreeMap<i64, PlayerRecord> = BTreeMap::new();
        let t = Records::read(db, "Player Tactical")?;
        for r in 0..t.len() {
            let player = t.int(r, "Player ID")?;
            let record = players.entry(player).or_default();
            record.player = player;
            record.deploy_points = t.int_or(r, "Deploy Res", 0)?;
        }
        type Counts = fn(&mut PlayerRecord) -> &mut BTreeMap<i64, i64>;
        let counts: [(&str, &str, Counts); 4] = [
            ("Player Tactical Units Destroyed", "Unit Type", |p| {
                &mut p.units_destroyed
            }),
            ("Player Tactical Units Lost", "Unit Type", |p| {
                &mut p.units_lost
            }),
            (
                "Player Tactical Buildings Destroyed",
                "Building Type",
                |p| &mut p.buildings_destroyed,
            ),
            ("Player Tactical Buildings Lost", "Building Type", |p| {
                &mut p.buildings_lost
            }),
        ];
        for (table, column, field) in counts {
            let t = Records::read(db, table)?;
            for r in 0..t.len() {
                let player = t.int(r, "Player")?;
                let record = players.entry(player).or_default();
                record.player = player;
                *field(record).entry(t.int(r, column)?).or_default() += t.int(r, "Number")?;
            }
        }
        self.players = players.into_values().collect();
        Ok(())
    }
}

/// The display name of an original savegame: its `.dsc` file name.
fn description(dir: &Path) -> Option<String> {
    fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let path = e.path();
        let dsc = path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("dsc"));
        dsc.then(|| path.file_stem()?.to_str().map(str::to_owned))
            .flatten()
    })
}

/// An original savegame slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalSave {
    pub dir: PathBuf,
    pub name: String,
    /// Saved during a battle (has a `SCENARIO.MDB`).
    pub battle: bool,
}

/// The original savegame slots (`SAVEGAME\savegame.NN\`) that hold a save.
pub fn original_saves(game: &GameData) -> Vec<OriginalSave> {
    let Ok(entries) = fs::read_dir(game.dir(Dir::SaveGame)) else {
        return Vec::new();
    };
    let mut saves: Vec<OriginalSave> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.to_ascii_lowercase().starts_with("savegame."))
        })
        .filter_map(|dir| {
            let name = description(&dir)?;
            let battle = find_case_insensitive(&dir, "SCENARIO.MDB").is_some();
            Some(OriginalSave { dir, name, battle })
        })
        .collect();
    saves.sort_by(|a, b| a.dir.cmp(&b.dir));
    saves
}

/// The shipped scenario `scenario` was most likely started from: same
/// title and size, fewest differing terrain cells (at most a quarter).
fn identify(game: &GameData, scenario: &Scenario) -> Result<Option<String>, DataError> {
    let title = &scenario.info.title;
    if title.is_empty() {
        return Ok(None);
    }
    let mut best: Option<(usize, String)> = None;
    for archive in SCENARIO_ARCHIVES {
        let Ok(bytes) = game.read(Dir::Scenarios, archive) else {
            continue;
        };
        let zip = Zip::parse(&bytes).map_err(|e| DataError::Archive(archive.to_owned(), e))?;
        for entry in zip.entries() {
            if !entry.name.to_ascii_lowercase().ends_with(".mdb") {
                continue;
            }
            let Ok(db) = zip
                .read(&entry.name)
                .map_err(|e| e.to_string())
                .and_then(|b| Database::open(b).map_err(|e| e.to_string()))
            else {
                continue;
            };
            let info = Records::read(&db, "Scenario Info")?;
            if info.is_empty() || info.text_or(0, "TitleEnglish")? != *title {
                continue;
            }
            let Ok(candidate) = Scenario::load(&db) else {
                continue;
            };
            if candidate.size != scenario.size {
                continue;
            }
            let diffs = candidate
                .cells
                .iter()
                .zip(&scenario.cells)
                .filter(|(a, b)| a.terrain != b.terrain)
                .count();
            if best.as_ref().is_none_or(|(d, _)| diffs < *d) {
                best = Some((diffs, entry.name.clone()));
            }
        }
    }
    Ok(best
        .filter(|(diffs, _)| *diffs * 4 <= scenario.cells.len())
        .map(|(_, name)| name))
}

#[derive(Serialize, Deserialize)]
struct TerrainChange {
    x: i64,
    y: i64,
    terrain: String,
}

/// The on-disk form of a [`Battle`].
#[derive(Serialize, Deserialize)]
struct SaveFile {
    format: String,
    version: u32,
    name: String,
    source: Source,
    size: usize,
    clock: Clock,
    difficulty: i64,
    turn_time: i64,
    auto_fire: bool,
    players: Vec<PlayerRecord>,
    /// Cells whose terrain differs from the source (e.g. burnt `X` variants).
    terrain: Vec<TerrainChange>,
    units: Vec<Unit>,
    buildings: Vec<Building>,
    walls: Vec<Surface>,
    roads: Vec<Surface>,
    ruins: Vec<Ruin>,
    mines: Vec<Mine>,
    order_targets: Vec<OrderTarget>,
}

fn parse(text: &str) -> Result<SaveFile, DataError> {
    #[derive(Deserialize)]
    struct Header {
        format: String,
        version: u32,
    }
    let header: Header =
        serde_json::from_str(text).map_err(|e| error(format!("not a battle save: {e}")))?;
    if header.format != FORMAT {
        return Err(error(format!("unknown save format {:?}", header.format)));
    }
    if header.version == 0 || header.version > VERSION {
        return Err(error(format!(
            "save version {} is not supported (this build reads 1 to {VERSION})",
            header.version
        )));
    }
    serde_json::from_str(text).map_err(|e| error(format!("invalid save: {e}")))
}
