#[allow(unused_imports)]
use crate::sources::*;
#[allow(unused_imports)]
use crate::plan::*;
#[allow(unused_imports)]
use crate::apply::*;
#[allow(unused_imports)]
use crate::diff::*;
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

/// Result of probing a source: reachable? and (if so) is the root a directory (tree compare) or a single
/// file (file diff)? Drives the picker's ✓/✗ connection check and folder-vs-file inference.
#[derive(Serialize)]
pub(crate) struct TestResult {
    pub(crate) ok: bool,
    pub(crate) is_dir: bool,
    pub(crate) message: String,
}

/// Probe a source's config: build it (for SFTP this connects + authenticates) and inspect its root.
/// Used by the picker to validate before enabling "Select", and to learn dir-vs-file.
#[tauri::command]
pub(crate) fn test_source(spec: SourceSpec) -> TestResult {
    let src = match build_source(&spec) {
        Ok(s) => s,
        Err(e) => {
            return TestResult {
                ok: false,
                is_dir: false,
                message: e,
            }
        }
    };
    match src.read_dir(&RelPath::root()) {
        Ok(_) => TestResult {
            ok: true,
            is_dir: true,
            message: "Connected".into(),
        },
        // Not a listable directory — maybe a single file (openable), otherwise a real error.
        Err(dir_err) => match src.open(&RelPath::root()) {
            Ok(_) => TestResult {
                ok: true,
                is_dir: false,
                message: "Connected (file)".into(),
            },
            Err(_) => TestResult {
                ok: false,
                is_dir: false,
                message: dir_err.to_string(),
            },
        },
    }
}

/// A single file on a source: which source + the relative path within it. The file-level commands take
/// this so they work uniformly across backends. *Files mode* roots an `fs` source at the file's parent
/// directory with `rel` = the filename; *folders mode* roots it at the compared folder with `rel` = the
/// tree entry's path — both reduce to `(source, rel)`, and SFTP/S3/… slot in by extending `SourceSpec`.
#[derive(Deserialize, Clone)]
pub(crate) struct FileRef {
    pub(crate) source: SourceSpec,
    pub(crate) rel: String,
}

/// Max bytes the text/hex views will load (the side-by-side and hex compare guard against huge files).
pub(crate) const TEXT_CAP: u64 = 2_000_000;
/// Cap for the large-file hunks-only mode. Files up to this size are scanned in full; beyond it
/// we stop after `DEFAULT_MAX_HUNKS` hunks. Both values are user-configurable via the warning dialog.
pub(crate) const LARGE_FILE_CAP: u64 = 10_000_000; // 10 MB
pub(crate) const DEFAULT_MAX_HUNKS: usize = 100;
pub(crate) const DEFAULT_CONTEXT_LINES: usize = 3;

/// Build a [`RelPath`] from a `/`-joined string (empty components ignored).
pub(crate) fn rel_from_str(s: &str) -> RelPath {
    let mut rel = RelPath::root();
    for c in s.split('/').filter(|c| !c.is_empty()) {
        rel = rel.child(c);
    }
    rel
}

/// Read a whole file's bytes from a [`ContentReader`] (zero-copy slice when the backend exposes one).
pub(crate) fn read_all(reader: &dyn ContentReader) -> Vec<u8> {
    if let Some(slice) = reader.as_slice() {
        return slice.to_vec();
    }
    let len = reader.len();
    let mut buf = vec![0u8; len as usize];
    let mut off = 0u64;
    while off < len {
        match reader.read_at(off, &mut buf[off as usize..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => off += n as u64,
        }
    }
    buf.truncate(off as usize);
    buf
}

/// The "file too large" error string for a read that exceeds `cap` (cap shown in decimal MB).
pub(crate) fn too_large(rel: &str, cap: u64) -> String {
    format!("{}: file too large (> {} MB)", rel, cap / 1_000_000)
}

/// Read up to `max` bytes from offset 0. Unlike [`read_all`] this **truncates** oversize files instead
/// of reading them whole — used for previews (hex view, large-file binary detection) where we only ever
/// want a bounded prefix and must never load (or reject) a huge file.
pub(crate) fn read_prefix(reader: &dyn ContentReader, max: usize) -> Vec<u8> {
    if let Some(slice) = reader.as_slice() {
        let n = slice.len().min(max);
        return slice[..n].to_vec();
    }
    let want = (reader.len() as usize).min(max);
    let mut buf = vec![0u8; want];
    let mut off = 0usize;
    while off < want {
        match reader.read_at(off as u64, &mut buf[off..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => off += n,
        }
    }
    buf.truncate(off);
    buf
}

/// Open a file via its source and read all bytes, rejecting anything larger than `cap`.
pub(crate) fn read_capped(file: &FileRef, cap: u64) -> Result<Vec<u8>, String> {
    let src = build_source(&file.source)?;
    let rel = rel_from_str(&file.rel);
    let reader = src.open(&rel).map_err(|e| e.to_string())?;
    if reader.len() > cap {
        return Err(too_large(&file.rel, cap));
    }
    Ok(read_all(reader.as_ref()))
}


/// `(mtime, created)` for the file at `rel`: list its parent directory and match the leaf name. Returns
/// `(None, None)` if it can't be determined (missing file, parent not listable, or `rel` is the root).
pub(crate) fn file_dates(src: &dyn Source, rel: &RelPath) -> (Option<i64>, Option<i64>) {
    let Some((leaf, parent_comps)) = rel.components().split_last() else {
        return (None, None);
    };
    let mut parent = RelPath::root();
    for c in parent_comps {
        parent = parent.child(c);
    }
    match src.read_dir(&parent) {
        Ok(entries) => entries
            .into_iter()
            .find(|e| &e.name == leaf)
            .map(|e| (e.mtime, e.created))
            .unwrap_or((None, None)),
        Err(_) => (None, None),
    }
}

/// An in-memory [`ContentReader`] over owned bytes, so `save_file` writes merged text through the same
/// [`SourceMut::copy_from`] path the sync engine uses — every writable backend gets saves for free.
pub(crate) struct BytesReader(pub(crate) Vec<u8>);

impl ContentReader for BytesReader {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError> {
        let start = offset.min(self.0.len() as u64) as usize;
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.0)
    }
}

/// Comparison options sent from the UI. The scan is always non-recursive (the UI loads levels lazily
/// or streams them in the background), so there is no `recursive` knob.
#[derive(Deserialize)]
pub(crate) struct CompareOpts {
    /// `full` | `quick` | `size` | `mtime` | `size-mtime` (anything else → `quick`).
    pub(crate) method: String,
    pub(crate) include: Vec<String>,
    pub(crate) exclude: Vec<String>,
}

pub(crate) fn parse_method(s: &str) -> CompareMethod {
    match s {
        "full" => CompareMethod::Full,
        "size" => CompareMethod::Size,
        "mtime" => CompareMethod::Mtime,
        "size-mtime" => CompareMethod::SizeAndMtime,
        _ => CompareMethod::Quick {
            large_file_threshold: DEFAULT_LARGE_FILE_THRESHOLD,
        },
    }
}


/// Shared backend state: a cache of connected read sources (so lazy level-by-level comparison reuses
/// ONE connection for the whole tree — a single SFTP/SSH session — instead of reconnecting on every
/// expand) plus the current comparison `token`. Bumping the token cancels stale background resolvers.
#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) cache: Mutex<HashMap<String, Arc<dyn Source>>>,
    /// The token of the comparison in progress. Arc so background threads can watch it for cancellation.
    pub(crate) token: Arc<AtomicU64>,
    /// Generation of the migrate-apply in progress. A running apply checks this between actions and
    /// stops if it no longer matches its own generation; `migrate_cancel` sets it to 0 to stop it.
    pub(crate) migrate_gen: Arc<AtomicU64>,
    /// Token of the migrate/sync plan being computed in the background. A newer preview bumps it; the
    /// worker thread checks it before emitting so a stale plan's result/progress is dropped.
    pub(crate) plan_token: Arc<AtomicU64>,
}

/// A single file's resolved verdict, pushed to the UI as it's computed in the background.
#[derive(Serialize, Clone)]
pub(crate) struct EntryResolved {
    pub(crate) token: u64,
    pub(crate) rel_path: Vec<String>,
    pub(crate) status: DiffStatus,
    pub(crate) detail: Option<String>,
}

/// A stable key identifying a source connection. Credentials are deliberately excluded (a session
/// uses one set), so the key never carries secrets and stays the same across every sub-path call.
pub(crate) fn cache_key(spec: &SourceSpec) -> String {
    kind_for(&spec.kind)
        .map(|k| k.cache_key(&spec.fields))
        .unwrap_or_else(|_| spec.kind.clone())
}

/// Return the cached source for `spec`, connecting (and caching) on a miss. The lock is held across
/// the connect so concurrent expands racing on the same miss connect once, not N times.
pub(crate) fn cached_source(state: &AppState, spec: &SourceSpec) -> Result<Arc<dyn Source>, String> {
    let key = cache_key(spec);
    let mut map = state
        .cache
        .lock()
        .map_err(|_| "source cache poisoned".to_string())?;
    if let Some(src) = map.get(&key) {
        return Ok(Arc::clone(src));
    }
    let src: Arc<dyn Source> = Arc::from(build_source(spec)?);
    map.insert(key, Arc::clone(&src));
    Ok(src)
}

/// Scan one directory level and stream its file verdicts. Returns the level's *listing* immediately
/// (directories on both sides → `skipped / "not descended"`; files on both sides → `skipped /
/// "comparing"` placeholders) so the UI paints at once, then resolves each file's real verdict on a
/// background thread, emitting an [`ENTRY_RESOLVED`] event per file. The thread stops early if the
/// `token` no longer matches the current comparison (a newer scan started). Child `rel_path`s are
/// already relative to the source roots, so the UI never re-bases paths.
pub(crate) fn run_scan(
    state: &AppState,
    app: &AppHandle,
    left: &SourceSpec,
    right: &SourceSpec,
    opts: &CompareOpts,
    rel: &RelPath,
    token: u64,
) -> Result<DiffReport, String> {
    let filters = FilterSet::new(&opts.include, &opts.exclude).map_err(|e| e.to_string())?;
    let method = parse_method(&opts.method);
    let cfg = CompareConfig {
        method,
        recursive: false,
        filters,
    };
    let l = cached_source(state, left)?;
    let r = cached_source(state, right)?;
    let report = engine_list_level(l.as_ref(), r.as_ref(), rel, &cfg).map_err(|e| e.to_string())?;

    // Files awaiting a verdict (placeholder "comparing"), to resolve in the background.
    let pending: Vec<(RelPath, EntryMeta, EntryMeta)> = report
        .root
        .children
        .iter()
        .filter(|c| !c.is_dir && c.detail.as_deref() == Some("comparing"))
        .filter_map(|c| match (&c.left, &c.right) {
            (Some(lm), Some(rm)) => Some((c.rel_path.clone(), lm.clone(), rm.clone())),
            _ => None,
        })
        .collect();

    if !pending.is_empty() {
        let app = app.clone();
        let cancel = Arc::clone(&state.token);
        std::thread::spawn(move || {
            let cfg = CompareConfig {
                method,
                recursive: false,
                filters: FilterSet::default(),
            };
            for (rel, lm, rm) in pending {
                if cancel.load(Ordering::SeqCst) != token {
                    return; // a newer comparison started — stop resolving the stale one.
                }
                let (status, detail) = engine_compare_file(l.as_ref(), r.as_ref(), &lm, &rm, &rel, &cfg);
                let _ = app.emit(
                    ENTRY_RESOLVED,
                    EntryResolved {
                        token,
                        rel_path: rel.components().to_vec(),
                        status,
                        detail,
                    },
                );
            }
        });
    }

    Ok(report)
}

/// Start a fresh comparison: bump the token (cancelling any stale background resolvers), drop cached
/// connections, then scan + stream the top level. Returns the top-level listing immediately.
#[tauri::command]
pub(crate) fn compare(
    state: State<AppState>,
    app: AppHandle,
    left: SourceSpec,
    right: SourceSpec,
    opts: CompareOpts,
    token: u64,
) -> Result<DiffReport, String> {
    state.token.store(token, Ordering::SeqCst);
    if let Ok(mut map) = state.cache.lock() {
        map.clear();
    }
    run_scan(&state, &app, &left, &right, &opts, &RelPath::root(), token)
}

/// Scan one directory level on demand (the user expanded a folder), reusing the cached connection.
/// Part of the same comparison — does NOT bump the token. Returns the listing + streams file verdicts.
#[tauri::command]
pub(crate) fn compare_level(
    state: State<AppState>,
    app: AppHandle,
    left: SourceSpec,
    right: SourceSpec,
    opts: CompareOpts,
    rel: String,
    token: u64,
) -> Result<DiffReport, String> {
    run_scan(&state, &app, &left, &right, &opts, &rel_from_str(&rel), token)
}
