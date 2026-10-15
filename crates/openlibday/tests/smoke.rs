//! Plays the `scripts/*.txt` test scripts headless. Skipped when
//! `LIBDAY_ROOT` is not set.

use std::path::Path;
use std::process::Command;

fn play(script: &str, done: &str) {
    if std::env::var_os("LIBDAY_ROOT").is_none() {
        eprintln!("skipping: LIBDAY_ROOT not set");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_openlibday"))
        .current_dir(&root)
        .args(["--headless", "--script", script])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains(done), "{stdout}");
}

#[test]
fn smoke_script() {
    play("scripts/smoke.txt", "smoke test passed");
}

#[test]
fn combat_script() {
    play("scripts/combat.txt", "combat test passed");
}

#[test]
fn mines_script() {
    play("scripts/mines.txt", "mines test passed");
}

#[test]
fn objectives_script() {
    play("scripts/objectives.txt", "objectives test passed");
}

#[test]
fn menu_script() {
    play("scripts/menu.txt", "menu test passed");
}

#[test]
fn testing_ground_script() {
    // Only present once tools/modding/make_testing_ground.py --install has run.
    let installed = std::env::var_os("LIBDAY_ROOT").is_some_and(|root| {
        Path::new(&root)
            .join("SCENS/SCEN1.ZIP.pre-testing-ground")
            .exists()
    });
    if !installed {
        eprintln!("skipping: Testing Ground not installed");
        return;
    }
    play("scripts/testing_ground.txt", "testing ground test passed");
    play("scripts/ai.txt", "ai test passed");
}
