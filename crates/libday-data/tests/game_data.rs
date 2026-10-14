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
