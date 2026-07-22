//! End-to-end CLI tests driving the `confold` binary.
// `assert_cmd::Command::cargo_bin` is deprecated in favor of a macro that requires a newer toolchain;
// the pinned (MSRV 1.84) version still uses this API. Silence the deprecation until the toolchain moves.
#![allow(deprecated)]

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn trees() -> (tempfile::TempDir, tempfile::TempDir) {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "same.txt", b"x");
    write(r.path(), "same.txt", b"x");
    write(l.path(), "diff.txt", b"left");
    write(r.path(), "diff.txt", b"right");
    write(l.path(), "only_left.txt", b"L");
    (l, r)
}

#[test]
fn text_output_lists_entries_and_summary() {
    let (l, r) = trees();
    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "compare",
            l.path().to_str().unwrap(),
            r.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("~ diff.txt"))
        .stdout(predicate::str::contains("< only_left.txt"))
        .stdout(predicate::str::contains("= same.txt"))
        .stdout(predicate::str::contains("1 different"));
}

#[test]
fn json_output_is_valid_and_has_summary() {
    let (l, r) = trees();
    let output = Command::cargo_bin("confold")
        .unwrap()
        .args([
            "compare",
            "--format",
            "json",
            l.path().to_str().unwrap(),
            r.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert_eq!(value["summary"]["different"], 1);
    assert_eq!(value["summary"]["left_only"], 1);
}

#[test]
fn fail_on_diff_sets_exit_code() {
    let (l, r) = trees();
    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "compare",
            "--fail-on-diff",
            l.path().to_str().unwrap(),
            r.path().to_str().unwrap(),
        ])
        .assert()
        .code(1);
}

#[test]
fn identical_trees_succeed_with_fail_on_diff() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "a.txt", b"same");
    write(r.path(), "a.txt", b"same");
    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "compare",
            "--fail-on-diff",
            l.path().to_str().unwrap(),
            r.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn missing_directory_errors_with_code_2() {
    let l = tempfile::tempdir().unwrap();
    Command::cargo_bin("confold")
        .unwrap()
        .args(["compare", l.path().to_str().unwrap(), "/no/such/dir/here"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error:"));
}

#[test]
fn capabilities_reports_semantic_protocol() {
    let output = Command::cargo_bin("confold")
        .unwrap()
        .args(["capabilities", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert_eq!(value["semantic_protocol_versions"], serde_json::json!([1]));
    assert!(value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "semantic prepare"));
}

#[test]
fn semantic_prepare_review_and_apply_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let left = dir.path().join("left.md");
    let right = dir.path().join("right.md");
    let bundle = dir.path().join("bundle.json");
    let proposal = dir.path().join("proposal.json");
    let merged = dir.path().join("merged.md");
    write(dir.path(), "left.md", b"# Left\n");
    write(dir.path(), "right.md", b"# Right\n");

    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "semantic",
            "prepare",
            "--left",
            left.to_str().unwrap(),
            "--right",
            right.to_str().unwrap(),
            "--output",
            bundle.to_str().unwrap(),
        ])
        .assert()
        .success();

    let bundle_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&bundle).unwrap()).expect("valid bundle JSON");
    let proposal_json = serde_json::json!({
        "schema_version": 1,
        "operation_id": bundle_json["operation_id"],
        "verdict": "merged",
        "summary": "Preserve both headings",
        "contributions": [
            {"source": "left", "intent": "Left heading", "disposition": "preserved"},
            {"source": "right", "intent": "Right heading", "disposition": "preserved"}
        ],
        "warnings": [],
        "result": "# Left and right\n"
    });
    fs::write(
        &proposal,
        serde_json::to_vec_pretty(&proposal_json).unwrap(),
    )
    .unwrap();

    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "semantic",
            "review",
            "--bundle",
            bundle.to_str().unwrap(),
            "--proposal",
            proposal.to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"applicable\": true"));

    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "semantic",
            "apply",
            "--bundle",
            bundle.to_str().unwrap(),
            "--proposal",
            proposal.to_str().unwrap(),
            "--output",
            merged.to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&merged).unwrap(), "# Left and right\n");
}

#[test]
fn semantic_apply_rejects_stale_input_and_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let left = dir.path().join("left.md");
    let right = dir.path().join("right.md");
    let bundle = dir.path().join("bundle.json");
    let proposal = dir.path().join("proposal.json");
    let merged = dir.path().join("merged.md");
    write(dir.path(), "left.md", b"left\n");
    write(dir.path(), "right.md", b"right\n");

    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "semantic",
            "prepare",
            "--left",
            left.to_str().unwrap(),
            "--right",
            right.to_str().unwrap(),
            "--output",
            bundle.to_str().unwrap(),
        ])
        .assert()
        .success();
    let bundle_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&bundle).unwrap()).unwrap();
    fs::write(
        &proposal,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "operation_id": bundle_json["operation_id"],
            "verdict": "prefer_left",
            "summary": "Left is authoritative",
            "contributions": [],
            "warnings": [],
            "result": null
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(&right, b"changed\n").unwrap();
    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "semantic",
            "apply",
            "--bundle",
            bundle.to_str().unwrap(),
            "--proposal",
            proposal.to_str().unwrap(),
            "--output",
            merged.to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("input changed after prepare"));

    fs::write(&right, b"right\n").unwrap();
    fs::write(&merged, b"do not replace\n").unwrap();
    Command::cargo_bin("confold")
        .unwrap()
        .args([
            "semantic",
            "apply",
            "--bundle",
            bundle.to_str().unwrap(),
            "--proposal",
            proposal.to_str().unwrap(),
            "--output",
            merged.to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("output already exists"));
    assert_eq!(fs::read_to_string(&merged).unwrap(), "do not replace\n");
}
