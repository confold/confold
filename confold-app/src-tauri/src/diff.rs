#[allow(unused_imports)]
use crate::sources::*;
#[allow(unused_imports)]
use crate::scan::*;
#[allow(unused_imports)]
use crate::plan::*;
#[allow(unused_imports)]
use crate::apply::*;
#[allow(unused_imports)]
use crate::{ENTRY_RESOLVED, MIGRATE_PROGRESS, MIGRATE_DONE, MIGRATE_PHASE, PLAN_PROGRESS, PLAN_READY};

#[allow(unused_imports)]
use confold_core::{
    compare as engine_compare, compare_at_with_progress as engine_compare_with_progress,
    compare_file as engine_compare_file, full_equal, list_level as engine_list_level,
    Capabilities, CompareConfig, CompareMethod, ContentReader, DiffEntry, DiffReport, DiffStatus,
    EntryMeta, FilterSet, LocalSource, RelPath, Source, SourceError, SourceMut,
    DEFAULT_LARGE_FILE_THRESHOLD,
};
#[allow(unused_imports)]
use confold_s3::{S3Config, S3Source};
#[allow(unused_imports)]
use confold_sftp::{SftpAuth, SftpConfig, SftpSource};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::{BTreeMap, HashMap};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicU64, Ordering};
#[allow(unused_imports)]
use std::sync::{Arc, LazyLock, Mutex};
#[allow(unused_imports)]
use tauri::{AppHandle, Emitter, State};
#[allow(unused_imports)]
use confold_sync::{apply as sync_apply, ActionOutcome, SyncAction, SyncOp};
#[allow(unused_imports)]
use confold_textdiff::{diff_text, diff_hunks, is_binary_bytes, FileDiff, FileDiffHunks};

/// Result of diffing two files: a text line/word diff, a binary byte-equality verdict, or a signal
/// that the file exceeds `TEXT_CAP` and should be opened via the large-file flow (`diff_file_large`).
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum DiffResult {
    Text { diff: FileDiff },
    Binary { identical: bool },
    TooLarge { left_size: u64, right_size: u64 },
}

/// Per-side metadata captured at read time: a content fingerprint (for conflict detection) plus the
/// line-ending style and whether the file ends with a newline — so a save round-trips byte-for-byte
/// (no lost trailing `\n`, no silent CRLF→LF) and an unchanged side stays equal on disk.
#[derive(Serialize)]
pub(crate) struct FileMeta {
    pub(crate) fp: String,
    pub(crate) eol: String,
    pub(crate) final_nl: bool,
    /// Modified / creation time (epoch ms) for the side-by-side header; `None` when unavailable.
    pub(crate) mtime: Option<i64>,
    pub(crate) created: Option<i64>,
}

/// `diff_file` payload: the diff plus per-side metadata (fingerprint + EOL/final-newline).
#[derive(Serialize)]
pub(crate) struct DiffFileResult {
    pub(crate) result: DiffResult,
    pub(crate) left: FileMeta,
    pub(crate) right: FileMeta,
}

/// A file's line ending for save reconstruction — CRLF if it contains any `\r\n`, else LF. (Mixed-EOL
/// files are normalised to CRLF on save; acceptable for our use.)
pub(crate) fn detect_eol(s: &str) -> &'static str {
    if s.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Read a file from an already-built source, treating a missing file as empty bytes (same contract
/// as the old `read_side` helper). The caller keeps ownership of the `Source` so it stays cached.
#[allow(clippy::type_complexity)]
pub(crate) fn read_side_cached(
    src: &dyn Source,
    rel: &RelPath,
    cap: u64,
) -> Result<(Vec<u8>, Option<i64>, Option<i64>), String> {
    let dates = file_dates(src, rel);
    match src.open(rel) {
        Ok(reader) => {
            if reader.len() > cap { return Err(too_large(&rel.to_string(), cap)); }
            Ok((read_all(reader.as_ref()), dates.0, dates.1))
        }
        Err(_) => Ok((Vec::new(), dates.0, dates.1)), // missing on this side → empty (one-sided view)
    }
}

/// Compute a side-by-side diff of two files. Text (UTF-8) → line/word diff; non-UTF-8/binary → a
/// byte-equality verdict. Guards file size (> TEXT_CAP → error). Uses the cached source so an
/// existing SFTP connection is reused instead of building a new one — avoids the full handshake.
#[tauri::command]
pub(crate) fn diff_file(state: State<AppState>, left: FileRef, right: FileRef) -> Result<DiffFileResult, String> {
    let src_l = cached_source(&state, &left.source)?;
    let src_r = cached_source(&state, &right.source)?;
    diff_file_inner(src_l.as_ref(), &rel_from_str(&left.rel), src_r.as_ref(), &rel_from_str(&right.rel))
}

/// Core of [`diff_file`], over already-built sources (so it's unit-testable without Tauri `State`).
pub(crate) fn diff_file_inner(src_l: &dyn Source, rel_l: &RelPath, src_r: &dyn Source, rel_r: &RelPath) -> Result<DiffFileResult, String> {
    // If either side exceeds TEXT_CAP, signal the UI to use the large-file flow instead of erroring.
    let left_size  = src_l.open(rel_l).map(|r| r.len()).unwrap_or(0);
    let right_size = src_r.open(rel_r).map(|r| r.len()).unwrap_or(0);
    if left_size > TEXT_CAP || right_size > TEXT_CAP {
        return Ok(DiffFileResult {
            result: DiffResult::TooLarge { left_size, right_size },
            left: FileMeta { fp: String::new(), eol: "\n".into(), final_nl: false, mtime: None, created: None },
            right: FileMeta { fp: String::new(), eol: "\n".into(), final_nl: false, mtime: None, created: None },
        });
    }
    let (lb, l_mtime, l_created) = read_side_cached(src_l, rel_l, TEXT_CAP)?;
    let (rb, r_mtime, r_created) = read_side_cached(src_r, rel_r, TEXT_CAP)?;
    let left_fp = fingerprint(&lb);
    let right_fp = fingerprint(&rb);
    let left_final_nl = lb.ends_with(b"\n");
    let right_final_nl = rb.ends_with(b"\n");
    let (result, left_eol, right_eol) = match (String::from_utf8(lb), String::from_utf8(rb)) {
        (Ok(l), Ok(r)) => {
            let le = detect_eol(&l).to_string();
            let re = detect_eol(&r).to_string();
            (
                DiffResult::Text {
                    diff: diff_text(&l, &r),
                },
                le,
                re,
            )
        }
        (l, r) => {
            let lb = l.map(String::into_bytes).unwrap_or_else(|e| e.into_bytes());
            let rb = r.map(String::into_bytes).unwrap_or_else(|e| e.into_bytes());
            (
                DiffResult::Binary {
                    identical: lb == rb,
                },
                "\n".to_string(),
                "\n".to_string(),
            )
        }
    };
    Ok(DiffFileResult {
        result,
        left: FileMeta {
            fp: left_fp,
            eol: left_eol,
            final_nl: left_final_nl,
            mtime: l_mtime,
            created: l_created,
        },
        right: FileMeta {
            fp: right_fp,
            eol: right_eol,
            final_nl: right_final_nl,
            mtime: r_mtime,
            created: r_created,
        },
    })
}

/// Result of the large-file diff: either a paginated text-hunks view or a binary verdict. Both carry
/// per-side metadata + real file sizes. One command, one round-trip — the UI discriminates on `kind`.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LargeDiffResult {
    TextHunks { hunks: FileDiffHunks, left: FileMeta, right: FileMeta, left_size: u64, right_size: u64 },
    Binary { identical: bool, left: FileMeta, right: FileMeta, left_size: u64, right_size: u64 },
}

/// The large-file diff (for files over `TEXT_CAP`). Detects binary vs text from a bounded prefix, then:
/// - **text** → reads up to `max_bytes` (default `LARGE_FILE_CAP`) and returns up to `max_hunks` hunk
///   regions from `start_hunk` onward (paginated "Load more"), each padded with `context_lines`.
/// - **binary** → returns a streaming byte-equality verdict (never loads the whole file); the hex view
///   reads its own bounded prefix via `hex_compare`.
/// Always read-only (editing needs the full file; see `diff_file`).
#[tauri::command]
pub(crate) fn diff_file_large(
    state: State<AppState>,
    left: FileRef,
    right: FileRef,
    max_bytes: Option<u64>,
    max_hunks: Option<usize>,
    context_lines: Option<usize>,
    start_hunk: Option<usize>,
) -> Result<LargeDiffResult, String> {
    let cap = max_bytes.unwrap_or(LARGE_FILE_CAP);
    let max_h = max_hunks.unwrap_or(DEFAULT_MAX_HUNKS).max(1);
    let ctx = context_lines.unwrap_or(DEFAULT_CONTEXT_LINES);
    let start = start_hunk.unwrap_or(0);

    let src_l = cached_source(&state, &left.source)?;
    let src_r = cached_source(&state, &right.source)?;
    let rel_l = rel_from_str(&left.rel);
    let rel_r = rel_from_str(&right.rel);
    let (l_mtime, l_created) = file_dates(src_l.as_ref(), &rel_l);
    let (r_mtime, r_created) = file_dates(src_r.as_ref(), &rel_r);

    // Open each side (missing → None, treated as empty / one-sided).
    let reader_l = src_l.open(&rel_l).ok();
    let reader_r = src_r.open(&rel_r).ok();
    let left_size  = reader_l.as_ref().map(|r| r.len()).unwrap_or(0);
    let right_size = reader_r.as_ref().map(|r| r.len()).unwrap_or(0);

    // Sniff binary from a bounded prefix (HEX_CAP is plenty; also serves as the binary fingerprint).
    let prefix = |r: &Option<Box<dyn ContentReader>>| r.as_ref().map(|r| read_prefix(r.as_ref(), HEX_CAP)).unwrap_or_default();
    let pre_l = prefix(&reader_l);
    let pre_r = prefix(&reader_r);

    if is_binary_bytes(&pre_l) || is_binary_bytes(&pre_r) {
        // Binary: equality via streaming full_equal (no full read). HexView loads its own prefix.
        let identical = match (&reader_l, &reader_r) {
            (Some(a), Some(b)) => full_equal(a.as_ref(), b.as_ref()).unwrap_or(false),
            _ => false, // one side missing → not identical
        };
        return Ok(LargeDiffResult::Binary {
            identical,
            left:  FileMeta { fp: fingerprint(&pre_l), eol: "\n".into(), final_nl: false, mtime: l_mtime, created: l_created },
            right: FileMeta { fp: fingerprint(&pre_r), eol: "\n".into(), final_nl: false, mtime: r_mtime, created: r_created },
            left_size, right_size,
        });
    }

    // Text: read content up to `cap` (truncated, not rejected — the warning says "up to N MB").
    let lb = reader_l.as_ref().map(|r| read_prefix(r.as_ref(), cap as usize)).unwrap_or_default();
    let rb = reader_r.as_ref().map(|r| read_prefix(r.as_ref(), cap as usize)).unwrap_or_default();
    let l_str = String::from_utf8_lossy(&lb).into_owned();
    let r_str = String::from_utf8_lossy(&rb).into_owned();
    let hunks = diff_hunks(&l_str, &r_str, ctx, max_h, start);
    Ok(LargeDiffResult::TextHunks {
        hunks,
        left:  FileMeta { fp: fingerprint(&lb), eol: detect_eol(&l_str).to_string(), final_nl: l_str.ends_with('\n'), mtime: l_mtime, created: l_created },
        right: FileMeta { fp: fingerprint(&rb), eol: detect_eol(&r_str).to_string(), final_nl: r_str.ends_with('\n'), mtime: r_mtime, created: r_created },
        left_size, right_size,
    })
}

/// A cheap, non-cryptographic content fingerprint (hex) for change detection — not a security hash.
/// `DefaultHasher`'s output isn't stable across Rust versions, so it must only be compared within one
/// process run (read → save), never persisted.
pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Re-diff two in-memory texts (used by the side-by-side after a copy-change/edit).
#[tauri::command]
pub(crate) fn diff_strings(left: String, right: String) -> FileDiff {
    diff_text(&left, &right)
}

/// Outcome of a save: written (with the new fingerprint) or refused because the file changed on disk.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SaveResult {
    Saved { fp: String },
    Conflict,
}

/// Write `contents` to `path` (save a merged side). Unless `force`, refuses with `Conflict` when the
/// file's current fingerprint differs from `expect` (i.e. it changed on disk since the UI read it).
#[tauri::command]
pub(crate) fn save_file(
    file: FileRef,
    contents: String,
    expect: Option<String>,
    force: bool,
) -> Result<SaveResult, String> {
    let src = build_source_mut(&file.source)?;
    let rel = rel_from_str(&file.rel);
    if !force {
        // Conflict only if the file EXISTS and changed since the UI read it. A missing target means we're
        // *creating* it (e.g. saving a one-sided file onto the other side) — that's never a conflict.
        if let Ok(reader) = src.open(&rel) {
            // The editor only opens files up to TEXT_CAP; if the on-disk file now exceeds it, it changed
            // out from under us — treat as a conflict rather than allocating an unbounded read for the fp.
            if reader.len() > TEXT_CAP {
                return Ok(SaveResult::Conflict);
            }
            if Some(fingerprint(&read_all(reader.as_ref()))) != expect {
                return Ok(SaveResult::Conflict);
            }
        }
    }
    src.copy_from(&rel, &BytesReader(contents.as_bytes().to_vec()))
        .map_err(|e| e.to_string())?;
    Ok(SaveResult::Saved {
        fp: fingerprint(contents.as_bytes()),
    })
}

// (R8) hex compare for binary files.
/// Max bytes returned per side for the hex view (the rest is summarised by `truncated` + the lengths).
pub(crate) const HEX_CAP: usize = 256 * 1024;

/// Bytes of two files for a hex compare. Each side is capped at `HEX_CAP`; `*_len` are the true lengths.
#[derive(Serialize)]
pub(crate) struct HexCompare {
    pub(crate) left: Vec<u8>,
    pub(crate) right: Vec<u8>,
    pub(crate) left_len: u64,
    pub(crate) right_len: u64,
    pub(crate) truncated: bool,
}

/// Read two files' first `HEX_CAP` bytes for a side-by-side hex comparison; `*_len` are the true sizes,
/// `truncated` if either exceeds the cap. Works for files of any size (prefix only — never rejects).
/// Uses the cached source so a binary opened from a folder compare reuses the existing connection.
#[tauri::command]
pub(crate) fn hex_compare(state: State<AppState>, left: FileRef, right: FileRef) -> Result<HexCompare, String> {
    let src_l = cached_source(&state, &left.source)?;
    let src_r = cached_source(&state, &right.source)?;
    Ok(hex_compare_inner(src_l.as_ref(), &rel_from_str(&left.rel), src_r.as_ref(), &rel_from_str(&right.rel)))
}

/// Core of [`hex_compare`], over already-built sources (unit-testable). Reads each side's first
/// `HEX_CAP` bytes (prefix only — never rejects a large file); `*_len` are the true sizes.
pub(crate) fn hex_compare_inner(src_l: &dyn Source, rel_l: &RelPath, src_r: &dyn Source, rel_r: &RelPath) -> HexCompare {
    let read_hex = |src: &dyn Source, rel: &RelPath| -> (Vec<u8>, u64) {
        match src.open(rel) {
            Ok(reader) => (read_prefix(reader.as_ref(), HEX_CAP), reader.len()),
            Err(_) => (Vec::new(), 0), // missing on this side → empty
        }
    };
    let (left_bytes, left_len) = read_hex(src_l, rel_l);
    let (right_bytes, right_len) = read_hex(src_r, rel_r);
    HexCompare {
        left: left_bytes,
        right: right_bytes,
        left_len,
        right_len,
        truncated: left_len > HEX_CAP as u64 || right_len > HEX_CAP as u64,
    }
}

/// Max image size we'll load into the webview for the image compare (raw bytes over IPC → blob URL).
pub(crate) const IMAGE_CAP: u64 = 25_000_000;

/// Read a file's raw bytes (for blob-URL image loading — same-origin, so the canvas isn't tainted and
/// pixel-diff works). Returned as an ArrayBuffer to the UI. Size-guarded (images can be larger than text).
#[tauri::command]
pub(crate) fn read_bytes(file: FileRef) -> Result<tauri::ipc::Response, String> {
    let bytes = read_capped(&file, IMAGE_CAP)?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// One configurable field of a source type — the UI renders a config form from these.
#[derive(Serialize)]
pub(crate) struct FieldSpec {
    /// Config key. Dotted (`auth.password`) for nested fields (the UI assembles the `SourceSpec`).
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
    /// Widget hint: `text` | `number` | `password` | `path` | `textarea` | `select`.
    pub(crate) kind: &'static str,
    pub(crate) required: bool,
    /// Secret material — the UI must mask it and avoid persisting it in plaintext.
    pub(crate) secret: bool,
    /// Default value, if any (e.g. SFTP port `22`).
    pub(crate) default: Option<&'static str>,
    /// Allowed values for a `select` field.
    pub(crate) options: Vec<&'static str>,
    /// Conditional display `"key=value"` — show only when another field has this value.
    pub(crate) show_when: Option<&'static str>,
}

impl FieldSpec {
    pub(crate) fn new(key: &'static str, label: &'static str, kind: &'static str, required: bool) -> Self {
        FieldSpec {
            key,
            label,
            kind,
            required,
            secret: false,
            default: None,
            options: Vec::new(),
            show_when: None,
        }
    }
    pub(crate) fn secret(mut self) -> Self {
        self.secret = true;
        self
    }
    pub(crate) fn default(mut self, d: &'static str) -> Self {
        self.default = Some(d);
        self
    }
    pub(crate) fn options(mut self, o: &[&'static str]) -> Self {
        self.options = o.to_vec();
        self
    }
    pub(crate) fn show_when(mut self, w: &'static str) -> Self {
        self.show_when = Some(w);
        self
    }
}

/// A source type the UI can offer: id, display name, icon, capabilities, and its config form.
#[derive(Serialize)]
pub(crate) struct SourceTypeInfo {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    /// Icon for the picker (emoji today) — sourced from the `SourceKind`, so the frontend needn't switch.
    pub(crate) icon: &'static str,
    /// What the type can do — gates which actions the UI offers. See `confold_vfs::Capabilities`.
    pub(crate) capabilities: Capabilities,
    pub(crate) fields: Vec<FieldSpec>,
}

/// Catalog of source types the UI can offer (drives the type picker + per-type config form), built from
/// the `SourceKind` registry. A new backend appears here automatically once registered.
#[tauri::command]
pub(crate) fn source_types() -> Vec<SourceTypeInfo> {
    REGISTRY
        .iter()
        .map(|k| SourceTypeInfo {
            id: k.id(),
            name: k.name(),
            icon: k.icon(),
            capabilities: k.capabilities(),
            fields: k.fields(),
        })
        .collect()
}
