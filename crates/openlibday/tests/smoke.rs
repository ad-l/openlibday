//! Plays `scripts/smoke.txt` and `scripts/combat.txt` headless. Skipped when
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
