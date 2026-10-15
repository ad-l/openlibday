//! Locates an original Liberation Day installation and resolves its files.
//!
//! Game data is never bundled with openlibday. The installation root is
//! taken from an explicit path or the `LIBDAY_ROOT` environment variable.
//! `EXE\LIBDAY.INI` `[Installation]` paths are honoured when they exist;
//! otherwise the standard directory names under the root are used, so a
//! copied installation keeps working. File lookups are case-insensitive so
//! the same data works on case-sensitive file systems.

pub mod battle;
pub mod campaign;
pub mod defs;
pub mod objective;
pub mod records;
pub mod scenario;

use std::fs;
use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use libday_formats::{Database, Ini, JetError, Zip, ZipError};
use thiserror::Error;

pub use battle::Battle;
pub use campaign::Campaign;
pub use defs::Definitions;
pub use scenario::{Scenario, SkirmishEntry};

pub const ROOT_ENV: &str = "LIBDAY_ROOT";

#[derive(Debug, Error)]
pub enum DataError {
    #[error("no game data configured; set {ROOT_ENV} to the Liberation Day install directory")]
    NotConfigured,
    #[error("{0} is not a Liberation Day installation (missing IMGS or GLOBALS)")]
    NotAnInstall(PathBuf),
    #[error("missing game file {0}")]
    Missing(PathBuf),
    #[error("{0}: {1}")]
    Database(String, JetError),
    #[error("{0}: {1}")]
    Archive(String, ZipError),
    #[error("{0}: {1}")]
    Strings(String, libday_formats::pe::PeError),
    #[error("{table}: {message}")]
    Invalid { table: String, message: String },
    #[error("savegame: {0}")]
    Savegame(String),
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Standard installation directories (`[Installation]` keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    AnmsUnit,
    Cache,
    ColAnms,
    Colonies,
    Encycl,
    Exe,
    Globals,
    Imgs,
    Interf,
    SaveGame,
    Scenarios,
    Sounds,
    Video,
}

impl Dir {
    /// `(INI key, default directory name)`.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Dir::AnmsUnit => ("anmsunit", "ANMSUNIT"),
            Dir::Cache => ("cache", "CACHE"),
            Dir::ColAnms => ("colanms", "COLANMS"),
            Dir::Colonies => ("colonies", "COLONIES"),
            Dir::Encycl => ("encycl", "ENCYCL"),
            Dir::Exe => ("exe", "EXE"),
            Dir::Globals => ("globals", "GLOBALS"),
            Dir::Imgs => ("imgs", "IMGS"),
            Dir::Interf => ("interf", "INTERF"),
            Dir::SaveGame => ("savegame", "SAVEGAME"),
            Dir::Scenarios => ("scenarios", "SCENS"),
            Dir::Sounds => ("sounds", "SOUNDS"),
            Dir::Video => ("video", "VIDEO"),
        }
    }
}

/// A located installation.
#[derive(Clone, Debug)]
pub struct GameData {
    root: PathBuf,
    ini: Ini,
}

/// Finds `name` in `dir` ignoring ASCII case.
pub fn find_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

impl GameData {
    /// Uses `LIBDAY_ROOT`.
    pub fn from_env() -> Result<Self, DataError> {
        match std::env::var_os(ROOT_ENV) {
            Some(root) if !root.is_empty() => Self::open(root),
            _ => Err(DataError::NotConfigured),
        }
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self, DataError> {
        let root = root.into();
        let has = |name| find_case_insensitive(&root, name).is_some_and(|p| p.is_dir());
        if !has("IMGS") || !has("GLOBALS") {
            return Err(DataError::NotAnInstall(root));
        }
        let ini = find_case_insensitive(&root, "EXE")
            .and_then(|exe| find_case_insensitive(&exe, "LIBDAY.INI"))
            .and_then(|path| fs::read(path).ok())
            .map(|bytes| Ini::parse_bytes(&bytes))
            .unwrap_or_default();
        Ok(Self { root, ini })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ini(&self) -> &Ini {
        &self.ini
    }

    pub fn dir(&self, dir: Dir) -> PathBuf {
        let (key, default) = dir.names();
        if let Some(path) = self.ini.get("Installation", key).map(PathBuf::from)
            && path.is_dir()
        {
            return path;
        }
        find_case_insensitive(&self.root, default).unwrap_or_else(|| self.root.join(default))
    }

    pub fn path(&self, dir: Dir, name: &str) -> Result<PathBuf, DataError> {
        let base = self.dir(dir);
        find_case_insensitive(&base, name).ok_or_else(|| DataError::Missing(base.join(name)))
    }

    pub fn read(&self, dir: Dir, name: &str) -> Result<Vec<u8>, DataError> {
        let path = self.path(dir, name)?;
        fs::read(&path).map_err(|source| DataError::Io { path, source })
    }

    /// Opens an Access database file.
    pub fn database(&self, dir: Dir, name: &str) -> Result<Database, DataError> {
        Database::open(self.read(dir, name)?).map_err(|e| DataError::Database(name.to_owned(), e))
    }

    /// Reads a scenario database by file name (e.g. `s1-1-1.mdb`): a loose
    /// file in the scenario directory wins, then the scenario archives.
    pub fn scenario_bytes(&self, name: &str) -> Result<Vec<u8>, DataError> {
        if let Ok(bytes) = self.read(Dir::Scenarios, name) {
            return Ok(bytes);
        }
        for archive in SCENARIO_ARCHIVES {
            let Ok(bytes) = self.read(Dir::Scenarios, archive) else {
                continue;
            };
            let zip = Zip::parse(&bytes).map_err(|e| DataError::Archive(archive.to_owned(), e))?;
            if zip.find(name).is_some() {
                return zip
                    .read(name)
                    .map_err(|e| DataError::Archive(archive.to_owned(), e));
            }
        }
        Err(DataError::Missing(self.dir(Dir::Scenarios).join(name)))
    }

    pub fn scenario(&self, name: &str) -> Result<Database, DataError> {
        Database::open(self.scenario_bytes(name)?)
            .map_err(|e| DataError::Database(name.to_owned(), e))
    }

    /// Scenario database names in the scenario archives.
    pub fn archived_scenarios(&self) -> Result<Vec<String>, DataError> {
        let mut names = Vec::new();
        for archive in SCENARIO_ARCHIVES {
            let Ok(bytes) = self.read(Dir::Scenarios, archive) else {
                continue;
            };
            let zip = Zip::parse(&bytes).map_err(|e| DataError::Archive(archive.to_owned(), e))?;
            names.extend(
                zip.entries()
                    .iter()
                    .filter(|e| e.name.to_ascii_lowercase().ends_with(".mdb"))
                    .map(|e| e.name.clone()),
            );
        }
        Ok(names)
    }

    /// UI strings from the English resource DLL (`EXE\LIBDAYEN.DLL`).
    pub fn strings(&self) -> Result<BTreeMap<u32, String>, DataError> {
        let name = "LIBDAYEN.DLL";
        libday_formats::pe::string_table(&self.read(Dir::Exe, name)?)
            .map_err(|e| DataError::Strings(name.to_owned(), e))
    }

    /// Bitmap resource `id` of the game executable (`EXE\LIBDAY.EXE`).
    pub fn exe_bitmap(&self, id: u32) -> Result<Option<libday_formats::pe::Bitmap>, DataError> {
        let name = "LIBDAY.EXE";
        libday_formats::pe::bitmap(&self.read(Dir::Exe, name)?, id)
            .map_err(|e| DataError::Strings(name.to_owned(), e))
    }
}

impl GameData {
    /// Definitions from `GLOBALS\GLOBAL.MDB`.
    pub fn definitions(&self) -> Result<Definitions, DataError> {
        Definitions::load(&self.database(Dir::Globals, "GLOBAL.MDB")?)
    }

    /// Loads a scenario by database name (see [`GameData::scenario_bytes`]).
    pub fn load_scenario(&self, name: &str) -> Result<Scenario, DataError> {
        Scenario::load(&self.scenario(name)?).map_err(|e| match e {
            DataError::Invalid { table, message } => DataError::Invalid {
                table: format!("{name}: {table}"),
                message,
            },
            other => other,
        })
    }

    /// The skirmish selector catalog (`SCENS\SKIRMISH.MDB`).
    pub fn skirmishes(&self) -> Result<Vec<SkirmishEntry>, DataError> {
        scenario::skirmish_catalog(&self.database(Dir::Scenarios, "SKIRMISH.MDB")?)
    }
}

/// Archives holding the shipped scenario databases, in lookup order.
pub const SCENARIO_ARCHIVES: [&str; 3] = ["SCEN1.ZIP", "SCEN2.ZIP", "DEATHM.ZIP"];
