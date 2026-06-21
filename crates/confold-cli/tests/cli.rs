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
