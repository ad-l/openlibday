//! Plays `scripts/smoke.txt` headless. Skipped when `LIBDAY_ROOT` is not set.

use std::path::Path;
use std::process::Command;

#[test]
fn smoke_script() {
    if std::env::var_os("LIBDAY_ROOT").is_none() {
        eprintln!("skipping: LIBDAY_ROOT not set");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_openlibday"))
        .current_dir(&root)
        .args(["--headless", "--script", "scripts/smoke.txt"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("smoke test passed"), "{stdout}");
}
