//! Integration tests for the local folder compare, including a differential check against native `cmp`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use confold_core::{
    compare, compare_at, compare_file, list_level, CompareConfig, CompareMethod, DiffEntry,
    DiffReport, DiffStatus, FilterSet, LocalSource, RelPath,
};

/// Write `bytes` to `dir/rel`, creating parent directories.
fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Flatten the report tree into `relative/path -> status`.
fn status_map(report: &DiffReport) -> HashMap<String, DiffStatus> {
    fn walk(entry: &DiffEntry, out: &mut HashMap<String, DiffStatus>) {
        if !entry.rel_path.is_root() {
            out.insert(entry.rel_path.to_string(), entry.status);
        }
        for child in &entry.children {
            walk(child, out);
        }
    }
    let mut out = HashMap::new();
    walk(&report.root, &mut out);
    out
}

fn run(left: &Path, right: &Path, cfg: &CompareConfig) -> DiffReport {
    compare(&LocalSource::new(left), &LocalSource::new(right), cfg).unwrap()
}

/// The top-level report child named `name` (for asserting status *and* detail).
fn child<'a>(report: &'a DiffReport, name: &str) -> &'a DiffEntry {
    report
        .root
        .children
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no child named {name}"))
}

/// Set a file's modified time to `secs` seconds past the epoch (deterministic mtime for the tests).
fn set_mtime(path: &Path, secs: u64) {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

#[test]
fn identical_trees_have_no_differences() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    for d in [l.path(), r.path()] {
        write(d, "a.txt", b"same");
        write(d, "sub/b.txt", b"nested");
    }
    let report = run(l.path(), r.path(), &CompareConfig::default());
    assert!(!report.has_differences());
    let map = status_map(&report);
    assert_eq!(map["a.txt"], DiffStatus::Identical);
    assert_eq!(map["sub/b.txt"], DiffStatus::Identical);
    assert_eq!(map["sub"], DiffStatus::Identical);
}

#[test]
fn detects_different_unique_and_nested() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "same.txt", b"x");
    write(r.path(), "same.txt", b"x");
    write(l.path(), "diff.txt", b"left");
    write(r.path(), "diff.txt", b"right!");
    write(l.path(), "only_left.txt", b"L");
    write(r.path(), "only_right.txt", b"R");
    write(l.path(), "tree/deep/leftish.txt", b"L");

    let report = run(l.path(), r.path(), &CompareConfig::default());
    let map = status_map(&report);

    assert_eq!(map["same.txt"], DiffStatus::Identical);
    assert_eq!(map["diff.txt"], DiffStatus::Different);
    assert_eq!(map["only_left.txt"], DiffStatus::LeftOnly);
    assert_eq!(map["only_right.txt"], DiffStatus::RightOnly);
    // a unique directory is enumerated recursively
    assert_eq!(map["tree"], DiffStatus::LeftOnly);
    assert_eq!(map["tree/deep/leftish.txt"], DiffStatus::LeftOnly);
    assert!(report.has_differences());
    assert_eq!(report.summary.different, 1);
    // left-only: only_left.txt + tree/ + tree/deep/ + tree/deep/leftish.txt
    assert_eq!(report.summary.left_only, 4);
    assert_eq!(report.summary.right_only, 1); // only_right.txt
}

#[test]
fn compare_at_loads_one_level_with_full_rel_paths() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    // A nested layout: the level we lazily load is `sub/`.
    for d in [l.path(), r.path()] {
        write(d, "sub/same.txt", b"x");
        write(d, "sub/deep/leaf.txt", b"y");
    }
    write(l.path(), "sub/diff.txt", b"left");
    write(r.path(), "sub/diff.txt", b"right!");

    // Non-recursive compare starting at `sub` — what the lazy UI does on expand.
    let cfg = CompareConfig {
        recursive: false,
        ..CompareConfig::default()
    };
    let start = RelPath::root().child("sub");
    let report = compare_at(
        &LocalSource::new(l.path()),
        &LocalSource::new(r.path()),
        &start,
        &cfg,
    )
    .unwrap();
    let map = status_map(&report);

    // Children are reported relative to the source ROOT (prefixed by `sub/`), not the sub-path.
    assert_eq!(map["sub/same.txt"], DiffStatus::Identical);
    assert_eq!(map["sub/diff.txt"], DiffStatus::Different);
    // A nested directory present on both sides is listed but not descended.
    assert_eq!(map["sub/deep"], DiffStatus::Skipped);
    let deep = report
        .root
        .children
        .iter()
        .find(|c| c.name == "deep")
        .unwrap();
    assert_eq!(deep.detail.as_deref(), Some("not descended"));
    assert!(deep.children.is_empty());
    // The leaf inside the non-descended dir is NOT loaded.
    assert!(!map.contains_key("sub/deep/leaf.txt"));
}

#[test]
fn list_level_defers_file_verdicts_then_compare_file_resolves_them() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "same.txt", b"x");
    write(r.path(), "same.txt", b"x");
    write(l.path(), "diff.txt", b"left");
    write(r.path(), "diff.txt", b"right!");
    write(l.path(), "only_left.txt", b"L");
    for d in [l.path(), r.path()] {
        write(d, "sub/inner.txt", b"z");
    }

    let cfg = CompareConfig {
        method: CompareMethod::Full,
        recursive: false,
        ..CompareConfig::default()
    };
    let ls = LocalSource::new(l.path());
    let rs = LocalSource::new(r.path());

    // Listing pass: file contents are NOT read — both-sides files are "comparing", not yet judged.
    let report = list_level(&ls, &rs, &RelPath::root(), &cfg).unwrap();
    let child = |name: &str| {
        report
            .root
            .children
            .iter()
            .find(|c| c.name == name)
            .unwrap()
    };

    let same = child("same.txt");
    assert_eq!(same.status, DiffStatus::Skipped);
    assert_eq!(same.detail.as_deref(), Some("comparing"));
    assert!(same.left.is_some() && same.right.is_some());
    // Items needing no content read are resolved immediately even in a listing pass.
    assert_eq!(child("only_left.txt").status, DiffStatus::LeftOnly);
    assert_eq!(child("sub").status, DiffStatus::Skipped);
    assert_eq!(child("sub").detail.as_deref(), Some("not descended"));

    // Resolving each "comparing" file via compare_file gives the real verdict.
    let resolve = |name: &str| {
        let c = child(name);
        compare_file(
            &ls,
            &rs,
            c.left.as_ref().unwrap(),
            c.right.as_ref().unwrap(),
            &c.rel_path,
            &cfg,
        )
        .0
    };
    assert_eq!(resolve("same.txt"), DiffStatus::Identical);
    assert_eq!(resolve("diff.txt"), DiffStatus::Different);
}

#[test]
fn filter_excludes_are_skipped() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "keep.txt", b"a");
    write(r.path(), "keep.txt", b"b"); // would be Different
    write(l.path(), "scratch.tmp", b"a");
    write(r.path(), "scratch.tmp", b"DIFFERENT"); // excluded ⇒ Skipped, not Different

    let cfg = CompareConfig {
        filters: FilterSet::new(&[], &["*.tmp".into()]).unwrap(),
        ..CompareConfig::default()
    };
    let report = run(l.path(), r.path(), &cfg);
    let map = status_map(&report);
    assert_eq!(map["keep.txt"], DiffStatus::Different);
    assert_eq!(map["scratch.tmp"], DiffStatus::Skipped);
}

#[test]
fn size_method_does_not_read_contents() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "f", b"AAAA");
    write(r.path(), "f", b"BBBB"); // same size, different content
    let cfg = CompareConfig {
        method: CompareMethod::Size,
        ..CompareConfig::default()
    };
    let report = run(l.path(), r.path(), &cfg);
    assert_eq!(status_map(&report)["f"], DiffStatus::Identical); // equal by size
}

#[test]
fn mtime_method_compares_modified_times() {
    // Same content, but the modified times decide the verdict under the `Mtime` method.
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "f", b"same");
    write(r.path(), "f", b"same");
    let cfg = CompareConfig {
        method: CompareMethod::Mtime,
        ..CompareConfig::default()
    };

    // Equal mtimes → Identical.
    set_mtime(&l.path().join("f"), 1500);
    set_mtime(&r.path().join("f"), 1500);
    assert_eq!(
        status_map(&run(l.path(), r.path(), &cfg))["f"],
        DiffStatus::Identical
    );

    // Differing mtimes → Different (even though the bytes match).
    set_mtime(&r.path().join("f"), 9000);
    let report = run(l.path(), r.path(), &cfg);
    assert_eq!(status_map(&report)["f"], DiffStatus::Different);
    assert_eq!(
        child(&report, "f").detail.as_deref(),
        Some("modified time differs")
    );
}

#[test]
fn size_and_mtime_method_requires_both_equal() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "f", b"AAAA");
    write(r.path(), "f", b"BBBB"); // same size, different bytes (size-mtime never reads content)
    let cfg = CompareConfig {
        method: CompareMethod::SizeAndMtime,
        ..CompareConfig::default()
    };

    // Same size + same mtime → Identical (content is not consulted).
    set_mtime(&l.path().join("f"), 1500);
    set_mtime(&r.path().join("f"), 1500);
    assert_eq!(
        status_map(&run(l.path(), r.path(), &cfg))["f"],
        DiffStatus::Identical
    );

    // Same size, different mtime → Different.
    set_mtime(&r.path().join("f"), 9000);
    assert_eq!(
        status_map(&run(l.path(), r.path(), &cfg))["f"],
        DiffStatus::Different
    );
}

#[cfg(unix)]
#[test]
fn symlinks_are_skipped_not_followed() {
    use std::os::unix::fs::symlink;
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "target.txt", b"x");
    write(r.path(), "target.txt", b"x");
    symlink("target.txt", l.path().join("link")).unwrap();
    symlink("target.txt", r.path().join("link")).unwrap();

    let report = run(l.path(), r.path(), &CompareConfig::default());
    let link = child(&report, "link");
    assert_eq!(link.status, DiffStatus::Skipped);
    assert_eq!(link.detail.as_deref(), Some("symlink (not followed)"));
}

#[test]
fn type_mismatch_file_vs_dir_is_different() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "x", b"i am a file");
    fs::create_dir(r.path().join("x")).unwrap(); // same name, but a directory on the right

    let report = run(l.path(), r.path(), &CompareConfig::default());
    let x = child(&report, "x");
    assert_eq!(x.status, DiffStatus::Different);
    assert_eq!(
        x.detail.as_deref(),
        Some("type differs (file vs directory)")
    );
}

#[test]
fn quick_method_samples_large_files() {
    // A small threshold forces the sampled path on a > 64 KiB file (so quick_equal probes head, tail
    // and the interior quarters via range_equal — the streaming sampler that had no coverage before).
    let big = vec![7u8; 128 * 1024]; // 7 (no NUL) → treated as text
    let cfg = CompareConfig {
        method: CompareMethod::Quick {
            large_file_threshold: 1000,
        },
        ..CompareConfig::default()
    };

    // Identical large files → Identical, flagged as sampled.
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "f", &big);
    write(r.path(), "f", &big);
    let report = run(l.path(), r.path(), &cfg);
    assert_eq!(child(&report, "f").status, DiffStatus::Identical);
    assert_eq!(
        child(&report, "f").detail.as_deref(),
        Some("identical by sampling (large file)")
    );

    // A difference at offset 0 (a sampled point) is caught.
    let mut diff_head = big.clone();
    diff_head[0] = 0;
    let r2 = tempfile::tempdir().unwrap();
    write(r2.path(), "f", &diff_head);
    let report2 = run(l.path(), r2.path(), &cfg);
    assert_eq!(child(&report2, "f").status, DiffStatus::Different);
}

/// Differential test: our `Full` verdict must agree with native `cmp -s` on many byte-pairs.
#[test]
fn full_compare_agrees_with_native_cmp() {
    let cases: Vec<(Vec<u8>, Vec<u8>)> = vec![
        (b"".to_vec(), b"".to_vec()),
        (b"abc".to_vec(), b"abc".to_vec()),
        (b"abc".to_vec(), b"abd".to_vec()), // differ at end
        (b"xbc".to_vec(), b"abc".to_vec()), // differ at start
        (b"abc".to_vec(), b"ab".to_vec()),  // different size
        (vec![0u8; 5000], vec![0u8; 5000]), // equal binary
        (
            {
                let mut v = vec![1u8; 5000];
                v[2500] = 9;
                v
            },
            vec![1u8; 5000],
        ), // differ in middle
        ((0..=255).collect(), (0..=255).collect()),
    ];

    for (i, (a, b)) in cases.iter().enumerate() {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        write(l.path(), "f", a);
        write(r.path(), "f", b);

        let cfg = CompareConfig {
            method: CompareMethod::Full,
            ..CompareConfig::default()
        };
        let report = run(l.path(), r.path(), &cfg);
        let ours_equal = status_map(&report)["f"] == DiffStatus::Identical;

        let cmp_equal = Command::new("cmp")
            .arg("-s")
            .arg(l.path().join("f"))
            .arg(r.path().join("f"))
            .status()
            .expect("cmp must be available")
            .success();

        assert_eq!(ours_equal, cmp_equal, "case {i}: engine vs cmp disagree");
    }
}
