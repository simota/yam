use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;

fn yam() -> Command {
    Command::cargo_bin("yam").expect("yam binary")
}

#[test]
fn extracts_raw_path_from_yaml() {
    yam()
        .args(["--raw", ".metadata.name", "testdata/sample.yaml"])
        .assert()
        .success()
        .stdout("my-config\n");
}

#[test]
fn converts_yaml_to_json() {
    yam()
        .args(["--json", "testdata/sample.yaml"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"apiVersion\": \"v1\""))
        .stdout(predicate::str::contains("\"port\": 5432"));
}

#[test]
fn renders_tree() {
    yam()
        .arg("testdata/sample.yaml")
        .assert()
        .success()
        .stdout(predicate::str::contains("metadata:"))
        .stdout(predicate::str::contains("└─ data:"));
}

#[test]
fn diff_returns_one_when_different() {
    yam()
        .args([
            "diff",
            "--summary",
            "testdata/config-dev.yaml",
            "testdata/config-prod.yaml",
        ])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Summary:"));
}

#[test]
fn fmt_sorts_keys() {
    let path = temp_file("yam-unsorted.yaml");
    fs::write(&path, "zebra: 1\napple: 2\nmango: 3\n").expect("write fixture");

    yam()
        .args(["fmt", "--sort-keys", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "apple: 2\nmango: 3\nzebra: 1\n",
        ));

    let _ = fs::remove_file(path);
}

#[test]
fn help_lists_interactive_mode() {
    yam()
        .args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--interactive"));
}

fn temp_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{}-{}", std::process::id(), name))
}
