//! Battle saves: openlibday round trips and original savegame import.
//! Skipped (with a note) when `LIBDAY_ROOT` is not set.

use libday_data::battle::{self, Source};
use libday_data::{Battle, Dir, GameData};

fn game() -> Option<GameData> {
    match GameData::from_env() {
        Ok(game) => Some(game),
        Err(err) => {
            eprintln!("skipping: {err}");
            None
        }
    }
}

fn round_trip(game: &GameData, battle: &Battle) {
    let base = battle.source.load(game).unwrap();
    let json = battle.to_json(&base).unwrap();
    assert_eq!(Battle::source_of(&json).unwrap(), battle.source);
    let restored = Battle::from_json(&json, base.clone()).unwrap();
    // Static tables and cell IDs come from the source, so compare the
    // saved state.
    assert_eq!(restored.to_json(&base).unwrap(), json);
}

#[test]
fn fresh_battle_round_trips_changes() {
    let Some(game) = game() else { return };
    let mut battle = Battle::start(&game, "s1-3-1.mdb").unwrap();
    assert_eq!(battle.clock.turn, 1);
    let s = &mut battle.scenario;
    s.cells[5].terrain = "NORX000".into();
    let unit = &mut s.units[1];
    unit.x += 1;
    unit.action_points = Some(7);
    unit.overwatch = false;
    s.units.remove(0);
    s.buildings.pop();
    battle.clock.turn = 4;

    let base = battle.source.load(&game).unwrap();
    let json = battle.to_json(&base).unwrap();
    let file: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(file["format"], battle::FORMAT);
    assert_eq!(file["version"], battle::VERSION);
    assert_eq!(file["terrain"].as_array().unwrap().len(), 1);
    assert_eq!(file["terrain"][0]["x"], 5);
    round_trip(&game, &battle);

    let dir = std::env::temp_dir().join(format!("openlibday-save-{}", std::process::id()));
    let path = dir.join("test.json");
    battle.save(&game, &path).unwrap();
    assert_eq!(Battle::open(&game, &path).unwrap(), battle);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_foreign_and_future_files() {
    let Some(game) = game() else { return };
    let base = game.load_scenario("s1-3-1.mdb").unwrap();
    let json = Battle::start(&game, "s1-3-1.mdb")
        .unwrap()
        .to_json(&base)
        .unwrap();
    let future = json.replacen("\"version\": 1", "\"version\": 99", 1);
    assert!(Battle::from_json(&future, base.clone()).is_err());
    let foreign = json.replacen(battle::FORMAT, "other", 1);
    assert!(Battle::from_json(&foreign, base.clone()).is_err());
    assert!(Battle::from_json("{}", base).is_err());
}

/// An original savegame slot, if this installation has it.
fn slot(game: &GameData, n: u32) -> Option<std::path::PathBuf> {
    let dir = game.dir(Dir::SaveGame).join(format!("savegame.{n:02}"));
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipping: no {}", dir.display());
        None
    }
}

#[test]
fn imports_original_battle_saves() {
    let Some(game) = game() else { return };
    if let Some(dir) = slot(&game, 1) {
        let battle = Battle::open(&game, &dir).unwrap();
        assert_eq!(battle.name, "end continent 1");
        assert_eq!(battle.source, Source::Scenario("s1-7-1.mdb".into()));
        assert_eq!(battle.scenario.info.title, "Scrap City");
        assert_eq!((battle.clock.turn, battle.clock.player), (9, 1));
        assert_eq!(battle.scenario.units.len(), 14);
        let start = game.load_scenario("s1-7-1.mdb").unwrap();
        let burnt = (battle.scenario.cells.iter().zip(&start.cells))
            .filter(|(a, b)| a.terrain != b.terrain)
            .count();
        assert_eq!(burnt, 149);
        let me = battle.players.iter().find(|p| p.player == 1).unwrap();
        assert_eq!(me.deploy_points, 182);
        assert_eq!(me.units_destroyed[&4], 26);
        assert_eq!(me.units_lost[&8], 5);
        round_trip(&game, &battle);
    }
    if let Some(dir) = slot(&game, 9) {
        let battle = Battle::import(&game, &dir).unwrap();
        assert_eq!(battle.source, Source::Scenario("s3-11-2.mdb".into()));
        assert_eq!(battle.clock.turn, 1);
        let u = battle.scenario.units.iter().find(|u| u.id == 370).unwrap();
        assert_eq!(
            (u.owner, u.armor, u.action_points),
            (1, Some(45), Some(250))
        );
        assert!(battle.scenario.next_unit_id() > 378);
        round_trip(&game, &battle);
    }
    let saves = battle::original_saves(&game);
    for save in saves.iter().filter(|s| !s.battle) {
        assert!(Battle::import(&game, &save.dir).is_err(), "{}", save.name);
    }
}
