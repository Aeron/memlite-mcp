use std::{path::Path, process::Command};

fn run_stats(project: &Path, db_dir: &Path, log_file: Option<&Path>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memlite"));
    command
        .args(["tool", "stats", "--format", "json"])
        .arg("--project-path")
        .arg(project)
        .arg("--db-dir")
        .arg(db_dir)
        .env("RUST_LOG", "trace");
    if let Some(log_file) = log_file {
        command.arg("--log-file").arg(log_file);
    }
    command.output().unwrap()
}

#[test]
fn help_and_version_use_memlite_name() {
    let output = Command::new(env!("CARGO_BIN_EXE_memlite"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Usage: memlite "), "{help}");

    // Checking the version name independently of the Cargo package name
    let output = Command::new(env!("CARGO_BIN_EXE_memlite"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("memlite {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn tool_json_output_omits_logs() {
    let project = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let output = run_stats(project.path(), db_dir.path(), None);

    assert!(
        output.status.success(),
        "tool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
}

#[test]
fn tool_delete_missing_memory_reports_false() {
    let project = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_memlite"))
        .args(["tool", "delete", "999", "--format", "json"])
        .arg("--project-path")
        .arg(project.path())
        .arg("--db-dir")
        .arg(db_dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, serde_json::json!({"deleted": false, "id": 999}));
}

#[test]
fn tool_honors_explicit_log_file() {
    let project = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let log_file = project.path().join("memlite.log");
    let output = run_stats(project.path(), db_dir.path(), Some(&log_file));

    assert!(
        output.status.success(),
        "tool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(log_file.is_file());
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
}
