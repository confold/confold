//! Benchmarks for the compare engine: a wide folder tree and large-file full vs quick comparison.

use std::fs;
use std::hint::black_box;
use std::path::Path;

use confold_core::{compare, CompareConfig, CompareMethod, LocalSource};
use criterion::{criterion_group, criterion_main, Criterion};
use tempfile::TempDir;

/// Build two sibling trees of `files` files spread across `dirs` subdirectories, all identical.
fn build_tree(files: usize, dirs: usize) -> (TempDir, TempDir) {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    for side in [l.path(), r.path()] {
        for d in 0..dirs {
            fs::create_dir_all(side.join(format!("d{d}"))).unwrap();
        }
        for i in 0..files {
            let rel = format!("d{}/f{i}.txt", i % dirs);
            fs::write(side.join(rel), format!("contents of file {i}\n").repeat(8)).unwrap();
        }
    }
    (l, r)
}

/// Write a `size`-byte file named `name` into both trees (identical content).
fn write_large(l: &Path, r: &Path, name: &str, size: usize) {
    let data = vec![0xABu8; size];
    fs::write(l.join(name), &data).unwrap();
    fs::write(r.join(name), &data).unwrap();
}

fn bench_folder_tree(c: &mut Criterion) {
    let (l, r) = build_tree(2000, 16);
    let left = LocalSource::new(l.path());
    let right = LocalSource::new(r.path());
    let cfg = CompareConfig {
        method: CompareMethod::Full,
        ..CompareConfig::default()
    };
    c.bench_function("folder_compare_2000_files_full", |b| {
        b.iter(|| black_box(compare(&left, &right, &cfg).unwrap()));
    });
}

fn bench_large_file(c: &mut Criterion) {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    let size = 64 * 1024 * 1024; // 64 MiB
    write_large(l.path(), r.path(), "big.bin", size);
    let left = LocalSource::new(l.path());
    let right = LocalSource::new(r.path());

    let full = CompareConfig {
        method: CompareMethod::Full,
        ..CompareConfig::default()
    };
    let quick = CompareConfig {
        method: CompareMethod::Quick {
            large_file_threshold: 4 * 1024 * 1024,
        },
        ..CompareConfig::default()
    };

    let mut group = c.benchmark_group("large_file_64MiB");
    group.bench_function("full", |b| {
        b.iter(|| black_box(compare(&left, &right, &full).unwrap()))
    });
    group.bench_function("quick_sampled", |b| {
        b.iter(|| black_box(compare(&left, &right, &quick).unwrap()))
    });
    group.finish();
}

criterion_group!(benches, bench_folder_tree, bench_large_file);
criterion_main!(benches);
