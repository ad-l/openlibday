//! Plays the `scripts/*.txt` test scripts headless. Skipped when
//! `LIBDAY_ROOT` is not set.

use std::path::Path;
use std::process::Command;

fn play(script: &str, done: &str) -> String {
    if std::env::var_os("LIBDAY_ROOT").is_none() {
        eprintln!("skipping: LIBDAY_ROOT not set");
        return String::new();
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
    stdout.into_owned()
}

#[test]
fn smoke_script() {
    play("scripts/smoke.txt", "smoke test passed");
}

/// A client following a host's command log ends in the host's state.
#[test]
fn replay_scripts() {
    let host = play("scripts/mp_host.txt", "hash ");
    let client = play("scripts/mp_client.txt", "hash ");
    let hash = |out: &str| {
        out.lines()
            .find(|l| l.starts_with("hash "))
            .map(String::from)
    };
    assert_eq!(hash(&host), hash(&client));
}

/// A host and a client joined in one process play a battle in step.
#[test]
fn loopback_script() {
    play("scripts/loopback.txt", "hash ");
}

/// A host and a client in one process play the co-op campaign in step.
#[test]
fn mp_campaign_script() {
    play("scripts/mp_campaign.txt", "mp campaign test passed");
}

/// A host and a client, two processes joined over TCP, play a battle in
/// step and end with the same state hash.
#[test]
fn tcp_scripts() {
    let host = std::thread::spawn(|| play("scripts/tcp_host.txt", "hash "));
    // The host listens before the client's first connection attempt fails.
    std::thread::sleep(std::time::Duration::from_secs(2));
    let client = play("scripts/tcp_client.txt", "hash ");
    let host = host.join().unwrap();
    if host.is_empty() {
        return;
    }
    let hash = |out: &str| {
        out.lines()
            .find(|l| l.starts_with("hash "))
            .map(String::from)
    };
    assert!(hash(&host).is_some());
    assert_eq!(hash(&host), hash(&client));
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
fn main_menu_script() {
    play("scripts/main_menu.txt", "main menu test passed");
}

#[test]
fn move_speed_script() {
    play("scripts/move_speed.txt", "move speed test passed");
}

#[test]
fn reinforce_script() {
    play("scripts/reinforce.txt", "reinforce test passed");
}

#[test]
fn automated_script() {
    play("scripts/automated.txt", "automated test passed");
}

#[test]
fn bridge_script() {
    play("scripts/bridge.txt", "bridge test passed");
}

#[test]
fn tech_script() {
    play("scripts/tech.txt", "tech test passed");
}

#[test]
fn scripted_support_script() {
    play(
        "scripts/scripted_support.txt",
        "scripted support test passed",
    );
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
    play("scripts/anti_air.txt", "anti-air test passed");
}
