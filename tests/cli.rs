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

const MANIFESTS: &str = "kind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 2\n  containers:\n  - name: app\n    image: nginx:1.25\n---\nkind: Service\nmetadata:\n  name: web\nspec:\n  ports:\n  - port: 80\n";

fn fixture(name: &str, content: &str) -> PathBuf {
    let path = temp_file(name);
    fs::write(&path, content).expect("write fixture");
    path
}

#[test]
fn queries_every_document_of_a_stream() {
    let path = fixture("yam-e2e-manifests.yaml", MANIFESTS);
    let file = path.to_str().unwrap();
    yam()
        .args(["-r", ".kind", file])
        .assert()
        .success()
        .stdout("Deployment\nService\n");
    yam()
        .args(["-r", "..image", file])
        .assert()
        .success()
        .stdout("nginx:1.25\n");
    yam()
        .args(["-d", "2", "-j", file])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\": \"Service\""))
        .stdout(predicate::str::contains("Deployment").not());
    yam()
        .args([".nope", file])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no match"));
    let _ = fs::remove_file(path);
}

#[test]
fn converts_between_json_and_yaml_keeping_order() {
    let path = fixture("yam-e2e-order.json", r#"{"z":1,"a":{"t":"true"}}"#);
    yam()
        .args(["--yaml", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout("z: 1\na:\n  t: 'true'\n");
    yam()
        .args(["--paths"])
        .write_stdin("a:\n  b: 1\n")
        .assert()
        .success()
        .stdout(".a.b = 1\n");
    let _ = fs::remove_file(path);
}

#[test]
fn fmt_keeps_documents_and_checks() {
    let path = fixture("yam-e2e-fmt.yaml", "a:   1\n---\nb:    2\n");
    let file = path.to_str().unwrap();
    yam()
        .args(["fmt", "--check", file])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(file));
    yam().args(["fmt", "-w", file]).assert().success();
    assert_eq!(fs::read_to_string(&path).unwrap(), "a: 1\n---\nb: 2\n");
    yam().args(["fmt", "--check", file]).assert().success();
    let _ = fs::remove_file(path);
}

#[test]
fn set_and_del_edit_files() {
    let path = fixture("yam-e2e-edit.yaml", MANIFESTS);
    let file = path.to_str().unwrap();
    yam()
        .args(["set", "-w", ".spec.replicas", "5", file])
        .assert()
        .success();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("replicas: 5"), "{text}");
    assert!(text.contains("kind: Service"), "{text}");
    assert_eq!(text.matches("replicas").count(), 1, "only the Deployment");

    yam()
        .args(["del", "-w", ".spec.containers[0]", file])
        .assert()
        .success();
    assert!(!fs::read_to_string(&path).unwrap().contains("nginx"));
    yam()
        .args(["del", ".nope", file])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("path not found"));
    yam()
        .args(["set", ".b", "x"])
        .write_stdin("a: 1\n")
        .assert()
        .success()
        .stdout("a: 1\nb: x\n");
    yam()
        .args(["set", "-w", ".b", "x"])
        .write_stdin("a: 1\n")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("stdin"));
    let _ = fs::remove_file(path);
}

#[test]
fn diff_exit_codes_and_json() {
    let a = fixture("yam-e2e-a.yaml", "x: 1\n");
    let b = fixture("yam-e2e-b.yaml", "x: 2\n");
    let (a_s, b_s) = (a.to_str().unwrap(), b.to_str().unwrap());
    yam().args(["diff", a_s, a_s]).assert().code(0);
    yam()
        .args(["diff", a_s, b_s])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("1 modified"));
    yam()
        .args(["diff", a_s, "/nonexistent.yaml"])
        .assert()
        .code(2);
    yam()
        .args(["diff", "--json", a_s, b_s])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"path\": \".x\""));
    yam()
        .args(["diff", "-s", b_s, "-"])
        .write_stdin("x: 2\n")
        .assert()
        .code(0);
    let _ = fs::remove_file(a);
    let _ = fs::remove_file(b);
}

#[test]
fn parse_errors_explain_the_cause() {
    yam()
        .write_stdin("a: [1, 2\n")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("failed to parse YAML:"));
}
