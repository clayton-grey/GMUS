use std::process::{Command, Output};

fn gmus() -> Command {
    Command::new(env!("CARGO_BIN_EXE_gmus"))
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn help_lists_documented_commands() {
    let output = gmus().arg("--help").output().unwrap();
    assert_success(&output);

    let stdout = stdout(&output);
    for expected in [
        "Usage: gmus",
        "scan",
        "art",
        "stats",
        "record-play",
        "play",
        "tui",
        "--db <DB>",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in\n{stdout}"
        );
    }
}

#[test]
fn record_play_help_documents_completed_flag() {
    let output = gmus().args(["record-play", "--help"]).output().unwrap();
    assert_success(&output);

    let stdout = stdout(&output);
    for expected in [
        "Usage: gmus record-play",
        "--duration-ms",
        "--completed",
        "BOOL",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in\n{stdout}"
        );
    }
}

#[test]
fn stats_accepts_db_override_and_initializes_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("custom.sqlite3");

    let output = gmus()
        .arg("--db")
        .arg(&db_path)
        .arg("stats")
        .output()
        .unwrap();
    assert_success(&output);

    let stdout = stdout(&output);
    assert!(db_path.is_file());
    assert!(dir.path().join("art").is_dir());
    for expected in [
        "tracks: 0",
        "locations: 0",
        "play events: 0",
        "completed plays: 0",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in\n{stdout}"
        );
    }
}
