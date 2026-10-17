use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;

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

#[test]
fn rejects_malformed_yaml() {
    let path = temp_file("yam-malformed.yaml");
    fs::write(&path, "a: [1, 2\n").expect("write fixture");

    yam()
        .arg(&path)
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("failed to parse YAML"));

    let _ = fs::remove_file(path);
}

#[test]
fn rejects_yaml_content_in_json_file() {
    let path = temp_file("yam-not-json.json");
    fs::write(&path, "a: 1\n").expect("write fixture");

    yam()
        .arg(&path)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("failed to parse JSON"));

    let _ = fs::remove_file(path);
}

#[test]
fn rejects_paths_that_do_not_resolve() {
    for (query, message) in [
        (".metadata.missing", "path not found: missing"),
        (
            ".metadata.name.x",
            "cannot traverse into scalar value at: x",
        ),
        (".data.features[9]", "array index out of bounds: 9"),
        (".data.features[x]", "invalid array index: x"),
        ("metadata", "path must start with '.': metadata"),
    ] {
        yam()
            .args([query, "testdata/sample.yaml"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn empty_stdin_is_a_null_document() {
    yam().write_stdin("").assert().success().stdout("null\n");
}

#[test]
fn closed_stdout_ends_output_without_error() {
    let path = temp_file("yam-large.yaml");
    let yaml: String = (0..50_000).map(|i| format!("k{i}: {i}\n")).collect();
    fs::write(&path, yaml).expect("write fixture");

    // Large enough to overflow the pipe buffer, so writes hit the closed pipe.
    for (args, code) in [
        (vec![path.to_str().unwrap()], 0),
        (vec!["--json", path.to_str().unwrap()], 0),
        (
            vec!["diff", path.to_str().unwrap(), "testdata/sample.yaml"],
            1,
        ),
    ] {
        let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("yam"))
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn yam");
        drop(child.stdout.take());
        let output = child.wait_with_output().expect("wait for yam");

        assert_eq!(output.status.code(), Some(code), "{args:?}");
        assert!(
            output.stderr.is_empty(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = fs::remove_file(path);
}

fn temp_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{}-{}", std::process::id(), name))
}
