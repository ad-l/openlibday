//! Data-gated checks against an installed copy of the game.
//! Skipped (with a note) when `LIBDAY_ROOT` is not set.

use libday_data::{Dir, GameData};
use libday_formats::Dbi;
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

/// Hash matching the Python editor reference: sorted names, then
/// `name \0 rgba` per sprite.
fn decode_all(game: &GameData, file: &str) -> (usize, String) {
    let bytes = game.read(Dir::Imgs, file).unwrap();
    let dbi = Dbi::parse(&bytes).unwrap();
    let mut names = dbi.names().to_vec();
    names.sort();
    let mut hash = Sha256::new();
    for name in &names {
        let sprite = dbi.sprite(name).unwrap_or_else(|e| panic!("{file}: {e}"));
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(sprite.to_rgba(&dbi.palette));
    }
    let digest = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    (dbi.len(), digest)
}

#[test]
fn dbi_matches_reference_decoder() {
    let Some(game) = game() else { return };
    let golden = [
        (
            "MINES.DBI",
            4,
            "59cac7d7b656df788fdf06ae9c291d3e3e5c0fae1a91cd4be62422ab24ef2fd9",
        ),
        (
            "ROADS.DBI",
            55,
            "7fa98224d6a731e277fda89b1415ff8d48027c5d118de29f863c24e6af65d85c",
        ),
        (
            "RUINS.DBI",
            137,
            "55646383e847003b02bedbdf7f81b863e66c3c7f74536e12eca99715b87189fc",
        ),
        (
            "SYMBOLS.DBI",
            138,
            "c13ffe3fa4b63714bd9b78b922c897c02925a648a0ca58411d661f05eaa450b4",
        ),
        (
            "TERRAINS.DBI",
            1119,
            "8a06ca1c53d4d6872f228383bd9160432d2a39fa1bd557931b57af141b170e83",
        ),
        (
            "UB.DBI",
            4225,
            "52b7bb71e7acd53a4c21b6985e933846529ce92cfea3aabe1d66155458757e3f",
        ),
        (
            "WALLS.DBI",
            140,
            "e80b7c4fc3abac48d008edef1bc33d7014370394e4d405275faafb99af2f2664",
        ),
    ];
    for (file, count, digest) in golden {
        assert_eq!(
            decode_all(&game, file),
            (count, digest.to_owned()),
            "{file}"
        );
    }
}

#[test]
fn listbox_dbi_decodes_with_38px_cells() {
    let Some(game) = game() else { return };
    let bytes = game.read(Dir::Imgs, "LISTBOX.DBI").unwrap();
    let dbi = Dbi::parse(&bytes).unwrap();
    assert_eq!(
        (dbi.len(), dbi.cell_width, dbi.cell_height, dbi.has_palette),
        (158, 38, 38, false)
    );
    for name in dbi.names() {
        dbi.sprite(name)
            .unwrap_or_else(|e| panic!("LISTBOX.DBI: {e}"));
    }
}

#[test]
fn definitions_and_skirmishes_load() {
    let Some(game) = game() else { return };
    let defs = game.definitions().unwrap();
    assert!(defs.terrains.len() > 100, "{}", defs.terrains.len());
    assert!(defs.terrain("BORE000").unwrap().paintable());
    assert!(!defs.buildings.is_empty() && !defs.units.is_empty());
    assert!(defs.units.keys().any(|&(_, tech)| tech == 10));
    assert_eq!(defs.roads[&0].image, "ROAL");
    let catalog = game.skirmishes().unwrap();
    assert_eq!(catalog.len(), 47);
    for entry in &catalog {
        let scenario = game.load_scenario(&entry.file_name).unwrap();
        assert!(scenario.size >= 10, "{}", entry.file_name);
        for cell in &scenario.cells {
            assert!(
                defs.terrain(&cell.terrain).is_some(),
                "{} {}",
                entry.file_name,
                cell.terrain
            );
        }
        for b in &scenario.buildings {
            assert!(defs.buildings.contains_key(&b.kind), "{}", entry.file_name);
        }
        for u in &scenario.units {
            assert!(
                defs.unit(u.kind).is_some(),
                "{} unit {}",
                entry.file_name,
                u.kind
            );
        }
        assert!(
            scenario.deployments.iter().any(|d| d.is_start_zone()),
            "{} has no deployment zone",
            entry.file_name
        );
        assert!(
            !scenario.info.available_units.is_empty(),
            "{} has no units to buy",
            entry.file_name
        );
        for &kind in &scenario.info.available_units {
            assert!(defs.unit(kind).is_some(), "{} buys {kind}", entry.file_name);
        }
    }
}

#[test]
fn skirmish_technology_data() {
    let Some(game) = game() else { return };
    let defs = game.definitions().unwrap();
    assert_eq!(defs.tech_points(150), 150);
    assert_eq!(defs.tech_points(301), 1505);
    assert_eq!(defs.tech_points(1000), 8000);
    assert_eq!(defs.tech_points(2000), 20000);
    let trooper = |tl| defs.unit_tech(2, tl).unwrap();
    assert_eq!((trooper(0).action_points, trooper(0).cost_to_next), (80, 5));
    assert_eq!(trooper(0).cost, 5);
    assert_eq!(trooper(5).cost_to_next, 0);
    assert_eq!(trooper(99).tech, trooper(5).tech);
    let s = game.load_scenario("s1-3-1.mdb").unwrap();
    assert_eq!(s.info.available_units, [2, 56, 8, 4, 3, 10]);
    assert_eq!(s.info.available_automated, [0, 7]);
    assert!(s.info.reinf_res >= 0);
}
