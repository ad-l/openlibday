//! Data-gated database, archive and string-table checks. Digests were
//! produced by this reader after it was verified cell-for-cell against
//! MDB Tools 1.0.1 (2,637 tables, 548,283 rows).

use libday_data::{Dir, GameData};
use libday_formats::{Database, Zip};
use sha2::{Digest, Sha256};

fn game() -> Option<GameData> {
    match GameData::from_env() {
        Ok(game) => Some(game),
        Err(err) => {
            eprintln!("skipping: {err}");
            None
        }
    }
}

/// `(user tables, total rows, digest)` over a canonical dump.
fn dump(db: &Database) -> (usize, usize, String) {
    let mut hash = Sha256::new();
    let (mut tables, mut rows) = (0, 0);
    let mut names: Vec<&str> = db.table_names().collect();
    names.sort_unstable();
    for name in names {
        let table = db.table(name).unwrap();
        hash.update(format!("{name}\n").as_bytes());
        for c in &table.columns {
            hash.update(format!("{}:{:?}:{}\n", c.name, c.kind, c.size).as_bytes());
        }
        let data = db.rows(&table).unwrap();
        for row in &data {
            hash.update(format!("{row:?}\n").as_bytes());
        }
        tables += 1;
        rows += data.len();
    }
    let digest = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    (tables, rows, digest)
}

/// Digest over every scenario database of an archive, in member order.
fn archive_dump(game: &GameData, archive: &str) -> (usize, usize, usize, String) {
    let bytes = game.read(Dir::Scenarios, archive).unwrap();
    let zip = Zip::parse(&bytes).unwrap();
    let mut hash = Sha256::new();
    let (mut dbs, mut tables, mut rows) = (0, 0, 0);
    for entry in zip.entries() {
        let data = zip.extract(entry).unwrap();
        if !entry.name.to_ascii_lowercase().ends_with(".mdb") {
            continue;
        }
        let (t, r, d) = dump(&Database::open(data).unwrap());
        hash.update(format!("{}:{d}\n", entry.name).as_bytes());
        dbs += 1;
        tables += t;
        rows += r;
    }
    let digest = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    (dbs, tables, rows, digest)
}

#[test]
fn databases_match_golden_dumps() {
    let Some(game) = game() else { return };
    let golden = [
        (
            Dir::Globals,
            "GLOBAL.MDB",
            34,
            2298,
            "d475f9b7b329024abf1ae2c68733c289247781eee1d0e8198810e892de640531",
        ),
        (
            Dir::Globals,
            "GAMEINFO.MDB",
            17,
            15,
            "629b915d1678b0f1d3daa0e4ee1af2bdd4ec3dd85c2062686e85e3b3a514578a",
        ),
        (
            Dir::Scenarios,
            "SKIRMISH.MDB",
            1,
            47,
            "1ad0cffb5dcd323931f3fd761d3df9d1fbd19646fc0c689f10f977c30a04243f",
        ),
        (
            Dir::Scenarios,
            "DEATHM.MDB",
            1,
            10,
            "845da3a1bd366f97e2d6c931acdca38379578750ad1207098a8c01a67c891c4d",
        ),
    ];
    let mut failed = false;
    for (dir, file, tables, rows, digest) in golden {
        let got = dump(&game.database(dir, file).unwrap());
        if got != (tables, rows, digest.to_owned()) {
            eprintln!("{file}: got {got:?}");
            failed = true;
        }
    }
    assert!(!failed);
}

#[test]
fn scenario_archives_match_golden_dumps() {
    let Some(game) = game() else { return };
    let golden = [
        (
            "SCEN1.ZIP",
            55,
            1211,
            260392,
            "5e1a897d8adec72031de5851831d79611d9f0d82474b1cf4b9cb09979acfa2d4",
        ),
        (
            "SCEN2.ZIP",
            50,
            1101,
            224352,
            "d370df6658d687a8b4bdf75f903011a8964b32de65035851b87d692167d517ef",
        ),
        (
            "DEATHM.ZIP",
            10,
            220,
            54745,
            "91d9dd6d548af36232613e12bc8ae5828d16ad035f66d0b9e4efc04792f3add7",
        ),
    ];
    let mut failed = false;
    for (archive, dbs, tables, rows, digest) in golden {
        let got = archive_dump(&game, archive);
        if got != (dbs, tables, rows, digest.to_owned()) {
            eprintln!("{archive}: got {got:?}");
            failed = true;
        }
    }
    assert!(!failed);
}

#[test]
fn scenario_lookup_reads_archived_databases() {
    let Some(game) = game() else { return };
    let names = game.archived_scenarios().unwrap();
    assert!(names.iter().any(|n| n.eq_ignore_ascii_case("s1-1-1.mdb")));
    let db = game.scenario("S1-1-1.MDB").unwrap();
    let info = db.table("Scenario Info").unwrap();
    let rows = db.rows(&info).unwrap();
    let title = info.column_index("TitleEnglish").unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0][title].as_str().is_some_and(|t| !t.is_empty()));
}

#[test]
fn ui_strings_load() {
    let Some(game) = game() else { return };
    let strings = game.strings().unwrap();
    assert_eq!(strings.len(), 312);
    assert_eq!(strings.get(&1).map(String::as_str), Some("Liberation Day"));
}

#[test]
fn exe_icon_bitmaps_load() {
    let Some(game) = game() else { return };
    for id in [181, 182, 183, 184, 185, 309] {
        let icon = game.exe_bitmap(id).unwrap().unwrap();
        assert_eq!((icon.width, icon.height), (25, 25), "bitmap {id}");
        assert_eq!(icon.rgba.len(), 25 * 25 * 4);
    }
    assert!(game.exe_bitmap(1).unwrap().is_none());
}
