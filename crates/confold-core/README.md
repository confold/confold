# confold-core

The Confold compare engine. Compares two directory trees exposed as
[`confold_vfs::Source`](../confold-vfs)s, classifies every item, compares file contents with a configurable
method, and returns a structured [`DiffReport`] (renderable as text or `serde` JSON).

This crate is the reusable core: it depends only on `confold-vfs` and never touches the filesystem
directly, so it can be embedded in a CLI, a GUI backend, or a server.

## Example

```rust
use confold_core::{compare, CompareConfig, CompareMethod, LocalSource, render_text};

let left = LocalSource::new("path/to/left");
let right = LocalSource::new("path/to/right");

let cfg = CompareConfig { method: CompareMethod::Full, ..CompareConfig::default() };
let report = compare(&left, &right, &cfg)?;

println!("{}", render_text(&report));
if report.has_differences() {
    // ... act on report.root / report.summary
}
# Ok::<(), confold_core::EngineError>(())
```

## Comparison methods

| `CompareMethod` | Reads contents? | Notes |
|-----------------|-----------------|-------|
| `Size` | no | equal iff byte sizes match |
| `Mtime` | no | equal iff modified times match |
| `SizeAndMtime` | no | both must match |
| `Full` | yes | byte-by-byte after a size pre-check; zero-copy slice compare on mmapped local files |
| `Quick { large_file_threshold }` | yes | `Full` up to the threshold; above it, samples head/tail + interior blocks (faster, with sampling uncertainty surfaced in the entry detail) |

## Design

The engine walks both trees in parallel (rayon), matches entries by relative path, triages by existence
and metadata, and compares contents per the chosen method — emitting a `DiffReport` (tree + summary) that
renders to text or JSON.
