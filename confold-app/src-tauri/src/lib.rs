//! Confold desktop backend (Tauri). Exposes the compare engine (`confold-core`) to the web UI.

use confold_core::{
    compare as engine_compare, compare_at_with_progress as engine_compare_with_progress,
    compare_file as engine_compare_file, full_equal, list_level as engine_list_level,
    Capabilities, CompareConfig, CompareMethod, ContentReader, DiffEntry, DiffReport, DiffStatus,
    EntryMeta, FilterSet, LocalSource, RelPath, Source, SourceError, SourceMut,
    DEFAULT_LARGE_FILE_THRESHOLD,
};
use confold_s3::{S3Config, S3Source};
use confold_sftp::{SftpAuth, SftpConfig, SftpSource};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tauri::{AppHandle, Emitter, State};

use confold_sync::{apply as sync_apply, ActionOutcome, SyncAction, SyncOp};
use confold_textdiff::{diff_text, diff_hunks, is_binary_bytes, FileDiff, FileDiffHunks};

/// A data source chosen in the UI: a registered kind id + flat config values. The generic wire DTO —
/// construction goes through the `SourceKind` registry below. Adding a backend never touches this type.
#[derive(Deserialize, Clone)]
struct SourceSpec {
    /// Registered source-kind id (e.g. `"fs"`, `"sftp"`), matching `SourceKind::id`.
    kind: String,
    /// Flat config values (dotted keys for nested fields, e.g. `auth.password`). Secrets live here too
    /// — never log them.
    #[serde(default)]
    fields: FieldValues,
}

fn default_sftp_port() -> u16 {
    22
}
fn default_sftp_root() -> String {
    "/".to_owned()
}

/// Connect an `SftpSource` from already-parsed SFTP config (shared by the read + write factories).
fn connect_sftp(
    host: &str,
    port: u16,
    username: &str,
    auth: SftpAuth,
    root: &str,
) -> Result<SftpSource, String> {
    SftpSource::connect(SftpConfig {
        host: host.to_owned(),
        port,
        username: username.to_owned(),
        auth,
        root: root.to_owned(),
    })
    .map_err(|e| e.to_string())
}

// ── Source plugins: descriptor + registry (Level 1) ─────────────────────────────────────────────
// Each source type is a `SourceKind` owning its identity: metadata + config form + capabilities + how
// to construct the backend. Adding a backend = implement this trait + register it below — no scattered
// `match` arms. The typed `SourceSpec` above is flattened to `FieldValues` via `to_fields()` (a
// transitional adapter; a later phase makes the wire format itself generic `{ kind, fields }`).

/// Flat config values for one source (dotted keys for nested fields, e.g. `auth.password`). Mirrors
/// what the UI's config form produces.
type FieldValues = BTreeMap<String, String>;

/// A registered source type: its identity, config form, capabilities, and how to construct it.
trait SourceKind: Send + Sync {
    /// Stable id (matches `SourceSpec`'s serde tag), e.g. `"fs"` / `"sftp"`.
    fn id(&self) -> &'static str;
    /// Human-readable name for the picker.
    fn name(&self) -> &'static str;
    /// Icon (emoji today) for the picker — replaces the frontend's per-type switch.
    fn icon(&self) -> &'static str;
    /// What this backend can do — gates the actions the UI offers.
    fn capabilities(&self) -> Capabilities;
    /// The config-form fields the UI renders.
    fn fields(&self) -> Vec<FieldSpec>;
    /// A stable, secret-free cache key for a connection (same identity → same key).
    fn cache_key(&self, f: &FieldValues) -> String;
    /// Build a read source from config.
    fn build_source(&self, f: &FieldValues) -> Result<Box<dyn Source>, String>;
    /// Build a writable source from config. Defaults to "read-only" for backends that don't implement it.
    fn build_source_mut(&self, _f: &FieldValues) -> Result<Box<dyn SourceMut>, String> {
        Err(format!("source type '{}' is read-only", self.id()))
    }
}

/// A required, non-empty config value, or a descriptive error.
fn required_field(f: &FieldValues, key: &str) -> Result<String, String> {
    f.get(key)
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or_else(|| format!("missing required field '{key}'"))
}

/// An optional config value (treats empty as absent).
fn optional_field(f: &FieldValues, key: &str) -> Option<String> {
    f.get(key).filter(|s| !s.is_empty()).cloned()
}

/// Local filesystem source.
struct FsKind;
impl SourceKind for FsKind {
    fn id(&self) -> &'static str {
        "fs"
    }
    fn name(&self) -> &'static str {
        "Local filesystem"
    }
    fn icon(&self) -> &'static str {
        "📁"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::FS_RW
    }
    fn fields(&self) -> Vec<FieldSpec> {
        vec![FieldSpec::new("root", "Folder or file path", "path", true)]
    }
    fn cache_key(&self, f: &FieldValues) -> String {
        format!("fs:{}", f.get("root").map(String::as_str).unwrap_or(""))
    }
    fn build_source(&self, f: &FieldValues) -> Result<Box<dyn Source>, String> {
        Ok(Box::new(LocalSource::new(required_field(f, "root")?)))
    }
    fn build_source_mut(&self, f: &FieldValues) -> Result<Box<dyn SourceMut>, String> {
        Ok(Box::new(LocalSource::new(required_field(f, "root")?)))
    }
}

/// Remote SFTP source.
struct SftpKind;
impl SftpKind {
    /// Connect from flat config (shared by the read + write factories — same as the typed path did).
    fn connect(&self, f: &FieldValues) -> Result<SftpSource, String> {
        let host = required_field(f, "host")?;
        let port = match optional_field(f, "port") {
            Some(p) => p
                .parse::<u16>()
                .map_err(|_| "port must be a number".to_string())?,
            None => default_sftp_port(),
        };
        let username = required_field(f, "username")?;
        let auth = sftp_auth_from_fields(f)?;
        let root = optional_field(f, "root").unwrap_or_else(default_sftp_root);
        connect_sftp(&host, port, &username, auth, &root)
    }
}
impl SourceKind for SftpKind {
    fn id(&self) -> &'static str {
        "sftp"
    }
    fn name(&self) -> &'static str {
        "SFTP"
    }
    fn icon(&self) -> &'static str {
        "🌐"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::FS_RW
    }
    fn fields(&self) -> Vec<FieldSpec> {
        vec![
            FieldSpec::new("host", "Host", "text", true),
            FieldSpec::new("port", "Port", "number", false).default("22"),
            FieldSpec::new("username", "Username", "text", true),
            FieldSpec::new("auth.method", "Authentication", "select", true)
                .options(&["password", "private_key"])
                .default("password"),
            FieldSpec::new("auth.password", "Password", "password", true)
                .secret()
                .show_when("auth.method=password"),
            FieldSpec::new("auth.pem", "Private key (PEM)", "textarea", true)
                .secret()
                .show_when("auth.method=private_key"),
            FieldSpec::new("auth.passphrase", "Key passphrase", "password", false)
                .secret()
                .show_when("auth.method=private_key"),
            FieldSpec::new("root", "Base directory", "path", false).default("/"),
        ]
    }
    fn cache_key(&self, f: &FieldValues) -> String {
        let g = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
        format!(
            "sftp:{}@{}:{}:{}",
            g("username"),
            g("host"),
            g("port"),
            g("root")
        )
    }
    fn build_source(&self, f: &FieldValues) -> Result<Box<dyn Source>, String> {
        Ok(Box::new(self.connect(f)?))
    }
    fn build_source_mut(&self, f: &FieldValues) -> Result<Box<dyn SourceMut>, String> {
        Ok(Box::new(self.connect(f)?))
    }
}

/// S3 / S3-compatible object storage source.
struct S3Kind;
impl S3Kind {
    /// Parse the flat config into an `S3Config` + in-bucket `prefix`.
    fn config(&self, f: &FieldValues) -> Result<(S3Config, String), String> {
        let config = S3Config {
            endpoint: optional_field(f, "endpoint"),
            region: optional_field(f, "region").unwrap_or_default(),
            bucket: required_field(f, "bucket")?,
            access_key_id: required_field(f, "access_key_id")?,
            secret_access_key: required_field(f, "secret_access_key")?,
        };
        Ok((config, optional_field(f, "prefix").unwrap_or_default()))
    }
}
impl SourceKind for S3Kind {
    fn id(&self) -> &'static str {
        "s3"
    }
    fn name(&self) -> &'static str {
        "S3 / S3-compatible"
    }
    fn icon(&self) -> &'static str {
        "☁️"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::FS_RW
    }
    fn fields(&self) -> Vec<FieldSpec> {
        vec![
            FieldSpec::new("bucket", "Bucket", "text", true),
            FieldSpec::new("prefix", "Prefix (path within the bucket)", "path", false),
            FieldSpec::new("region", "Region", "text", false).default("us-east-1"),
            FieldSpec::new(
                "endpoint",
                "Endpoint (blank for AWS; set for MinIO / S3-compatible)",
                "text",
                false,
            ),
            FieldSpec::new("access_key_id", "Access key ID", "text", true),
            FieldSpec::new("secret_access_key", "Secret access key", "password", true).secret(),
        ]
    }
    fn cache_key(&self, f: &FieldValues) -> String {
        let g = |k: &str| f.get(k).map(String::as_str).unwrap_or("");
        // Secret-free identity (no secret access key): access key id @ endpoint / bucket / prefix.
        format!(
            "s3:{}@{}/{}/{}",
            g("access_key_id"),
            g("endpoint"),
            g("bucket"),
            g("prefix")
        )
    }
    fn build_source(&self, f: &FieldValues) -> Result<Box<dyn Source>, String> {
        let (config, prefix) = self.config(f)?;
        Ok(Box::new(
            S3Source::connect(config, prefix).map_err(|e| e.to_string())?,
        ))
    }
    fn build_source_mut(&self, f: &FieldValues) -> Result<Box<dyn SourceMut>, String> {
        let (config, prefix) = self.config(f)?;
        Ok(Box::new(
            S3Source::connect(config, prefix).map_err(|e| e.to_string())?,
        ))
    }
}

/// Build an `SftpAuth` from flat config (`auth.method` + the matching secret fields).
fn sftp_auth_from_fields(f: &FieldValues) -> Result<SftpAuth, String> {
    match f.get("auth.method").map(String::as_str) {
        Some("password") => Ok(SftpAuth::Password(required_field(f, "auth.password")?)),
        Some("private_key") => Ok(SftpAuth::PrivateKey {
            pem: required_field(f, "auth.pem")?,
            passphrase: optional_field(f, "auth.passphrase"),
        }),
        Some(other) => Err(format!("unknown auth method '{other}'")),
        None => Err("missing required field 'auth.method'".to_string()),
    }
}

/// The registered source types. Adding a backend = add its `SourceKind` here.
static REGISTRY: LazyLock<Vec<Box<dyn SourceKind>>> =
    LazyLock::new(|| vec![Box::new(FsKind), Box::new(SftpKind), Box::new(S3Kind)]);

/// Look up a registered source kind by id.
fn kind_for(id: &str) -> Result<&'static dyn SourceKind, String> {
    REGISTRY
        .iter()
        .find(|k| k.id() == id)
        .map(|k| &**k)
        .ok_or_else(|| format!("unknown source type '{id}'"))
}

/// Build a read source from a spec (via the registry).
fn build_source(spec: &SourceSpec) -> Result<Box<dyn Source>, String> {
    kind_for(&spec.kind)?.build_source(&spec.fields)
}

/// Build a writable source from a spec — errors if the type can't be written to.
fn build_source_mut(spec: &SourceSpec) -> Result<Box<dyn SourceMut>, String> {
    kind_for(&spec.kind)?.build_source_mut(&spec.fields)
}

/// Result of probing a source: reachable? and (if so) is the root a directory (tree compare) or a single
/// file (file diff)? Drives the picker's ✓/✗ connection check and folder-vs-file inference.
#[derive(Serialize)]
struct TestResult {
    ok: bool,
    is_dir: bool,
    message: String,
}

/// Probe a source's config: build it (for SFTP this connects + authenticates) and inspect its root.
/// Used by the picker to validate before enabling "Select", and to learn dir-vs-file.
#[tauri::command]
fn test_source(spec: SourceSpec) -> TestResult {
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
struct FileRef {
    source: SourceSpec,
    rel: String,
}

/// Max bytes the text/hex views will load (the side-by-side and hex compare guard against huge files).
const TEXT_CAP: u64 = 2_000_000;
/// Cap for the large-file hunks-only mode. Files up to this size are scanned in full; beyond it
/// we stop after `DEFAULT_MAX_HUNKS` hunks. Both values are user-configurable via the warning dialog.
const LARGE_FILE_CAP: u64 = 10_000_000; // 10 MB
const DEFAULT_MAX_HUNKS: usize = 100;
const DEFAULT_CONTEXT_LINES: usize = 3;

/// Build a [`RelPath`] from a `/`-joined string (empty components ignored).
fn rel_from_str(s: &str) -> RelPath {
    let mut rel = RelPath::root();
    for c in s.split('/').filter(|c| !c.is_empty()) {
        rel = rel.child(c);
    }
    rel
}

/// Read a whole file's bytes from a [`ContentReader`] (zero-copy slice when the backend exposes one).
fn read_all(reader: &dyn ContentReader) -> Vec<u8> {
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
fn too_large(rel: &str, cap: u64) -> String {
    format!("{}: file too large (> {} MB)", rel, cap / 1_000_000)
}

/// Read up to `max` bytes from offset 0. Unlike [`read_all`] this **truncates** oversize files instead
/// of reading them whole — used for previews (hex view, large-file binary detection) where we only ever
/// want a bounded prefix and must never load (or reject) a huge file.
fn read_prefix(reader: &dyn ContentReader, max: usize) -> Vec<u8> {
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
fn read_capped(file: &FileRef, cap: u64) -> Result<Vec<u8>, String> {
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
fn file_dates(src: &dyn Source, rel: &RelPath) -> (Option<i64>, Option<i64>) {
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
struct BytesReader(Vec<u8>);

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
struct CompareOpts {
    /// `full` | `quick` | `size` | `mtime` | `size-mtime` (anything else → `quick`).
    method: String,
    include: Vec<String>,
    exclude: Vec<String>,
}

fn parse_method(s: &str) -> CompareMethod {
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

/// Event name: one file's streamed verdict (status + detail) for a `list_level` placeholder.
const ENTRY_RESOLVED: &str = "entry-resolved";
/// Event name: progress of one applied migrate action (its concrete outcome).
const MIGRATE_PROGRESS: &str = "migrate-progress";
/// Event name: migrate apply finished (complete or cancelled). Carries the final tally.
const MIGRATE_DONE: &str = "migrate-done";
/// Event name: a long migrate phase started (M2 re-verification / origin-delete) — for UI status.
const MIGRATE_PHASE: &str = "migrate-phase";
/// Event name: live "examined N items" counter while a migrate/sync plan is being computed.
const PLAN_PROGRESS: &str = "plan-progress";
/// Event name: a migrate/sync plan finished computing — carries the action list (or an error).
const PLAN_READY: &str = "plan-ready";

/// Shared backend state: a cache of connected read sources (so lazy level-by-level comparison reuses
/// ONE connection for the whole tree — a single SFTP/SSH session — instead of reconnecting on every
/// expand) plus the current comparison `token`. Bumping the token cancels stale background resolvers.
#[derive(Default)]
struct AppState {
    cache: Mutex<HashMap<String, Arc<dyn Source>>>,
    /// The token of the comparison in progress. Arc so background threads can watch it for cancellation.
    token: Arc<AtomicU64>,
    /// Generation of the migrate-apply in progress. A running apply checks this between actions and
    /// stops if it no longer matches its own generation; `migrate_cancel` sets it to 0 to stop it.
    migrate_gen: Arc<AtomicU64>,
    /// Token of the migrate/sync plan being computed in the background. A newer preview bumps it; the
    /// worker thread checks it before emitting so a stale plan's result/progress is dropped.
    plan_token: Arc<AtomicU64>,
}

/// A single file's resolved verdict, pushed to the UI as it's computed in the background.
#[derive(Serialize, Clone)]
struct EntryResolved {
    token: u64,
    rel_path: Vec<String>,
    status: DiffStatus,
    detail: Option<String>,
}

/// A stable key identifying a source connection. Credentials are deliberately excluded (a session
/// uses one set), so the key never carries secrets and stays the same across every sub-path call.
fn cache_key(spec: &SourceSpec) -> String {
    kind_for(&spec.kind)
        .map(|k| k.cache_key(&spec.fields))
        .unwrap_or_else(|_| spec.kind.clone())
}

/// Return the cached source for `spec`, connecting (and caching) on a miss. The lock is held across
/// the connect so concurrent expands racing on the same miss connect once, not N times.
fn cached_source(state: &AppState, spec: &SourceSpec) -> Result<Arc<dyn Source>, String> {
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
fn run_scan(
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
fn compare(
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
fn compare_level(
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

/// Which categories of difference a migration (origin → destination) acts on. The destructive ones
/// are opt-in in the UI, each behind a warning. The MOVE step (delete origin after a verified-complete
/// migration, M2) is not a plan category — it is requested at apply time via `migrate_apply`'s
/// `deleteOrigin` flag and realised by [`move_origin`], so it does not belong here.
#[derive(Deserialize, Clone, Copy)]
struct MigrateFlags {
    /// Copy items present only in origin → destination (create them on the destination).
    copy_new: bool,
    /// Overwrite destination files whose content differs from origin.
    overwrite_different: bool,
    /// Delete items present only in destination — mirror, makes the destination match origin.
    delete_extra: bool,
}

/// Why a migration action was generated — carries the original diff status so the UI can show
/// `→ copy` vs `→ override` vs `✕ delete` per side without re-running the comparison.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MigrateReason {
    /// Item is only in origin — will be created in destination.
    New,
    /// Item exists on both sides but content differs — will overwrite destination.
    Different,
    /// Item is only in destination — will be deleted from destination.
    Extra,
    /// MOVE semantics (M2): an origin item deleted after being verified identical in destination.
    /// Only emitted on the post-apply origin-delete progress, never produced by `collect_migrate`.
    Moved,
}

/// A single leaf operation within a directory copy — one file or one empty directory. Used to drive
/// per-file cancellation without exposing the full subtree to the UI.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct LeafOp {
    rel_path: Vec<String>,
    /// `true` only for empty directories (which need an explicit `create_dir_all` since no file copy
    /// would trigger it automatically).
    is_dir: bool,
}

impl LeafOp {
    fn to_sync_action(&self, op: SyncOp) -> SyncAction {
        let mut rel = RelPath::root();
        for component in &self.rel_path {
            rel = rel.child(component);
        }
        SyncAction { rel_path: rel, op, is_dir: self.is_dir }
    }
}

/// A planned migrate operation enriched with its origin diff status (`reason`), so the UI can
/// display what happens on each side without re-running the comparison.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct MigrateAction {
    /// Path components, relative to the compared roots.
    rel_path: Vec<String>,
    op: SyncOp,
    is_dir: bool,
    reason: MigrateReason,
    /// Total items inside a directory action (files + sub-dirs, recursive). 1 for a file action.
    /// Lets the UI show "dir • 47 items" without listing every leaf in the plan.
    item_count: usize,
    /// Pre-expanded leaf operations for directory COPY actions (enables per-file cancellation).
    /// The UI never sees this detail — it's only used by `run_migrate`. Empty for file actions and
    /// delete actions (deletes stay atomic: cancelling a partial delete leaves inconsistent state).
    #[serde(default)]
    leaves: Vec<LeafOp>,
}

impl MigrateAction {
    fn to_sync_action(&self) -> SyncAction {
        let mut rel = RelPath::root();
        for component in &self.rel_path {
            rel = rel.child(component);
        }
        SyncAction {
            rel_path: rel,
            op: self.op,
            is_dir: self.is_dir,
        }
    }
}

/// Count all items (files + directories) in a subtree recursively. Used for directory actions so the
/// UI can show "dir • 47 items" without listing every leaf in the plan.
fn count_items(node: &DiffEntry) -> usize {
    node.children.iter().fold(0, |acc, c| {
        acc + 1 + if c.is_dir { count_items(c) } else { 0 }
    })
}

/// Expand a directory subtree into per-leaf copy operations so `run_migrate` can cancel between
/// individual files. Files and empty directories are added directly; non-empty directories are
/// recursed into (their files will create parent directories automatically via `copy_from`).
fn collect_copy_leaves(node: &DiffEntry, out: &mut Vec<LeafOp>) {
    for child in &node.children {
        if !child.is_dir {
            out.push(LeafOp { rel_path: child.rel_path.components().to_vec(), is_dir: false });
        } else if child.children.is_empty() {
            // Empty directory: needs an explicit create since no file copy will trigger mkdir.
            out.push(LeafOp { rel_path: child.rel_path.components().to_vec(), is_dir: true });
        } else {
            collect_copy_leaves(child, out);
        }
    }
}

/// Expand a directory subtree into per-leaf DELETE operations in bottom-up order (children before
/// parents), so cancellation fires between individual files and dirs are only removed once empty.
/// The caller is responsible for appending the root dir itself after calling this.
fn collect_delete_leaves(node: &DiffEntry, out: &mut Vec<LeafOp>) {
    for child in &node.children {
        if !child.is_dir {
            out.push(LeafOp { rel_path: child.rel_path.components().to_vec(), is_dir: false });
        } else {
            collect_delete_leaves(child, out); // children first
            out.push(LeafOp { rel_path: child.rel_path.components().to_vec(), is_dir: true }); // then the dir (now empty)
        }
    }
}

/// Build a copy action for a diff entry in a given direction. A directory expands into per-leaf copies
/// (`leaves`); a file is a single op. Shared by Migrate and Sync — `op` carries the direction.
fn copy_subtree_action(c: &DiffEntry, op: SyncOp, reason: MigrateReason) -> MigrateAction {
    let mut leaves = Vec::new();
    if c.is_dir {
        collect_copy_leaves(c, &mut leaves);
    }
    MigrateAction {
        rel_path: c.rel_path.components().to_vec(),
        op,
        is_dir: c.is_dir,
        reason,
        item_count: if c.is_dir { count_items(c) } else { 1 },
        leaves,
    }
}

/// Build a delete action for a diff entry on a given side. A directory expands bottom-up (children then
/// the now-empty dir) so cancellation is per-leaf. Shared by Migrate (delete-extra) and Sync.
fn delete_subtree_action(c: &DiffEntry, op: SyncOp) -> MigrateAction {
    let mut leaves = Vec::new();
    if c.is_dir {
        collect_delete_leaves(c, &mut leaves); // children bottom-up
        leaves.push(LeafOp { rel_path: c.rel_path.components().to_vec(), is_dir: true }); // root dir last
    }
    MigrateAction {
        rel_path: c.rel_path.components().to_vec(),
        op,
        is_dir: c.is_dir,
        reason: MigrateReason::Extra,
        item_count: if c.is_dir { count_items(c) } else { 1 },
        leaves,
    }
}

/// Walk a (recursive) diff tree and collect migrate actions enriched with their reason and item count.
/// One action per top-level diff: a one-sided directory becomes a single recursive copy/delete; a
/// "different" directory is a container, so we recurse into it and only act on its diverging children.
fn collect_migrate(node: &DiffEntry, flags: MigrateFlags, out: &mut Vec<MigrateAction>) {
    for c in &node.children {
        match c.status {
            DiffStatus::LeftOnly => {
                if flags.copy_new {
                    out.push(copy_subtree_action(c, SyncOp::CopyLeftToRight, MigrateReason::New));
                }
            }
            DiffStatus::Different => {
                if c.is_dir {
                    collect_migrate(c, flags, out);
                } else if flags.overwrite_different {
                    out.push(copy_subtree_action(c, SyncOp::CopyLeftToRight, MigrateReason::Different));
                }
            }
            DiffStatus::RightOnly => {
                if flags.delete_extra {
                    out.push(delete_subtree_action(c, SyncOp::DeleteRight));
                }
            }
            DiffStatus::Identical | DiffStatus::Skipped | DiffStatus::Error => {}
        }
    }
}

// ── Sync: bidirectional reconciliation (S1) ──────────────────────────────────────────────────────
//
// Sync generalises Migrate. The two "trust" flags declare which side(s) are authoritative; direction
// follows from that (no stored baseline needed). When exactly one side is trusted it behaves like a
// one-directional Migrate (and `delete_diffs` mirrors `delete_extra`); when both are trusted it's a
// bidirectional union and `conflict_rule` decides each "different on both sides" file. Reuses
// `MigrateAction` (the `op` carries direction; `reason` classifies New/Different/Extra for the counts)
// and the whole apply path (`migrate_apply` with `delete_origin = false`).

/// How a "different on both sides" conflict is resolved when BOTH sides are trusted.
#[derive(Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum ConflictRule {
    /// Never auto-resolve — leave every conflict for the user to handle in Compare afterward. Default:
    /// it's the least destructive (no file is ever auto-overwritten on a content conflict).
    #[default]
    Manual,
    /// The side with the later mtime wins; a tie or unknown mtime is left for manual resolution.
    Newer,
    /// The larger file wins; equal size is left for manual resolution.
    Larger,
}

/// Which side(s) a Sync run trusts, plus how it handles the untrusted side's extras and conflicts. At
/// least one side must be trusted (the UI enforces it; neither-trusted yields no actions).
#[derive(Deserialize, Clone, Copy)]
struct SyncFlags {
    /// Left source is authoritative — its content propagates to the right.
    trust_left: bool,
    /// Right source is authoritative — its content propagates to the left.
    trust_right: bool,
    /// When exactly one side is trusted, also delete the untrusted side's extra items.
    #[serde(default)]
    delete_diffs: bool,
    /// Conflict resolution when BOTH sides are trusted (ignored otherwise).
    #[serde(default)]
    conflict_rule: ConflictRule,
}

/// Decide which way a single "different" file is copied, or `None` to leave it for manual resolution
/// (manual rule, neither side trusted, or a tie under newer/larger).
fn resolve_conflict(c: &DiffEntry, flags: SyncFlags) -> Option<SyncOp> {
    match (flags.trust_left, flags.trust_right) {
        (true, false) => Some(SyncOp::CopyLeftToRight), // left is the sole authority
        (false, true) => Some(SyncOp::CopyRightToLeft), // right is the sole authority
        (false, false) => None,                         // no authority (UI prevents this)
        (true, true) => match flags.conflict_rule {
            ConflictRule::Manual => None,
            ConflictRule::Newer => {
                pick_by(c.left.as_ref().and_then(|m| m.mtime), c.right.as_ref().and_then(|m| m.mtime))
            }
            ConflictRule::Larger => {
                pick_by(c.left.as_ref().map(|m| m.size), c.right.as_ref().map(|m| m.size))
            }
        },
    }
}

/// Copy from the side with the strictly-greater metric; `None` on a tie or any missing value.
fn pick_by<T: Ord>(left: Option<T>, right: Option<T>) -> Option<SyncOp> {
    match (left, right) {
        (Some(l), Some(r)) if l > r => Some(SyncOp::CopyLeftToRight),
        (Some(l), Some(r)) if r > l => Some(SyncOp::CopyRightToLeft),
        _ => None,
    }
}

/// Walk a (recursive) diff tree and collect bidirectional sync actions per the trust flags + conflict
/// rule. Unresolved conflicts (manual rule, or a tie) are omitted — the post-apply re-compare surfaces
/// them for manual resolution in Compare. Mirrors `collect_migrate`'s container/recursion handling.
fn collect_sync(node: &DiffEntry, flags: SyncFlags, out: &mut Vec<MigrateAction>) {
    for c in &node.children {
        match c.status {
            DiffStatus::LeftOnly => {
                if flags.trust_left {
                    out.push(copy_subtree_action(c, SyncOp::CopyLeftToRight, MigrateReason::New));
                } else if flags.trust_right && flags.delete_diffs {
                    out.push(delete_subtree_action(c, SyncOp::DeleteLeft));
                }
            }
            DiffStatus::RightOnly => {
                if flags.trust_right {
                    out.push(copy_subtree_action(c, SyncOp::CopyRightToLeft, MigrateReason::New));
                } else if flags.trust_left && flags.delete_diffs {
                    out.push(delete_subtree_action(c, SyncOp::DeleteRight));
                }
            }
            DiffStatus::Different => {
                if c.is_dir {
                    collect_sync(c, flags, out); // container: recurse into diverging children
                } else if let Some(op) = resolve_conflict(c, flags) {
                    out.push(MigrateAction {
                        rel_path: c.rel_path.components().to_vec(),
                        op,
                        is_dir: false,
                        reason: MigrateReason::Different,
                        item_count: 1,
                        leaves: vec![],
                    });
                }
            }
            DiffStatus::Identical | DiffStatus::Skipped | DiffStatus::Error => {}
        }
    }
}

/// Plan a migration: run a FULL recursive comparison (so nothing is decided on metadata alone, and big
/// files stream rather than load) and return the enriched actions (with their `reason`) for the enabled
/// `flags`. The UI then previews them (`plan_actions`) and, on confirm, applies them. Direction is fixed
/// origin → dest.
/// Live "examined N items" counter while a plan is computed (throttled — one per 256 entries).
#[derive(Serialize, Clone)]
struct PlanProgress {
    token: u64,
    examined: u64,
}

/// Result of a migrate/sync plan computation, streamed to the UI when the background compare finishes.
#[derive(Serialize, Clone)]
struct PlanReady {
    token: u64,
    /// `"migrate"` or `"sync"` — so the UI opens the right plan modal.
    flow: &'static str,
    actions: Vec<MigrateAction>,
    /// Set instead of `actions` when the compare failed.
    error: Option<String>,
}

/// Run a migrate/sync plan computation OFF the main thread (called from a plain `std::thread` so the
/// S3/SFTP sources' own `block_on` is safe — see memory `tauri-heavy-commands-async`). Streams a throttled
/// `PLAN_PROGRESS` counter as the recursive compare walks, then emits `PLAN_READY` (actions or error).
/// A newer preview bumps `plan_token`, so a superseded plan drops its progress + result silently.
#[allow(clippy::too_many_arguments)]
fn run_plan(
    app: &AppHandle,
    left: &dyn Source,
    right: &dyn Source,
    cfg: &CompareConfig,
    flow: &'static str,
    token: u64,
    plan_token: &Arc<AtomicU64>,
    collect: impl Fn(&DiffEntry) -> Vec<MigrateAction>,
) {
    let examined = AtomicU64::new(0);
    let progress = || {
        let n = examined.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_multiple_of(256) && plan_token.load(Ordering::SeqCst) == token {
            let _ = app.emit(PLAN_PROGRESS, PlanProgress { token, examined: n });
        }
    };
    let result = engine_compare_with_progress(left, right, &RelPath::root(), cfg, &progress);
    if plan_token.load(Ordering::SeqCst) != token {
        return; // a newer preview superseded this one
    }
    let payload = match result {
        Ok(report) => {
            // Emit the exact final count so the throttled counter lands on the true total instead of
            // freezing at the last multiple of 256 (the tail < 256 entries emit no progress event).
            let _ = app.emit(PLAN_PROGRESS, PlanProgress { token, examined: examined.load(Ordering::Relaxed) });
            PlanReady { token, flow, actions: collect(&report.root), error: None }
        }
        Err(e) => PlanReady { token, flow, actions: Vec::new(), error: Some(e.to_string()) },
    };
    let _ = app.emit(PLAN_READY, payload);
}

#[tauri::command]
fn migrate_actions(
    state: State<AppState>,
    app: AppHandle,
    left: SourceSpec,
    right: SourceSpec,
    opts: CompareOpts,
    flags: MigrateFlags,
    token: u64,
) -> Result<(), String> {
    state.plan_token.store(token, Ordering::SeqCst);
    let filters = FilterSet::new(&opts.include, &opts.exclude).map_err(|e| e.to_string())?;
    let cfg = CompareConfig {
        method: parse_method(&opts.method),
        recursive: true,
        filters,
    };
    let l = cached_source(&state, &left)?;
    let r = cached_source(&state, &right)?;
    let plan_token = Arc::clone(&state.plan_token);
    std::thread::spawn(move || {
        run_plan(&app, l.as_ref(), r.as_ref(), &cfg, "migrate", token, &plan_token, |root| {
            let mut actions = Vec::new();
            collect_migrate(root, flags, &mut actions);
            actions
        });
    });
    Ok(())
}

/// Plan a bidirectional sync: a FULL recursive comparison, then collect the directional actions per the
/// trust flags + conflict rule. Returns `MigrateAction`s (reused — `op` carries the direction); the UI
/// previews them and applies via `migrate_apply` (with `delete_origin = false`).
#[tauri::command]
fn sync_actions(
    state: State<AppState>,
    app: AppHandle,
    left: SourceSpec,
    right: SourceSpec,
    opts: CompareOpts,
    flags: SyncFlags,
    token: u64,
) -> Result<(), String> {
    state.plan_token.store(token, Ordering::SeqCst);
    let filters = FilterSet::new(&opts.include, &opts.exclude).map_err(|e| e.to_string())?;
    let cfg = CompareConfig {
        method: parse_method(&opts.method),
        recursive: true,
        filters,
    };
    let l = cached_source(&state, &left)?;
    let r = cached_source(&state, &right)?;
    let plan_token = Arc::clone(&state.plan_token);
    std::thread::spawn(move || {
        run_plan(&app, l.as_ref(), r.as_ref(), &cfg, "sync", token, &plan_token, |root| {
            let mut actions = Vec::new();
            collect_sync(root, flags, &mut actions);
            actions
        });
    });
    Ok(())
}

/// Progress of one concrete migrate operation (a `SyncAction` may expand into several — e.g. a
/// directory copy yields one per descendant). Streamed to the UI as each completes.
#[derive(Serialize, Clone)]
struct MigrateProgress {
    /// The generation this run belongs to, so the UI ignores events from a superseded apply.
    generation: u64,
    rel_path: Vec<String>,
    op: SyncOp,
    /// The original diff reason — distinguishes `→ copy` (new) from `→ override` (different) in the UI.
    reason: MigrateReason,
    ok: bool,
    error: Option<String>,
}

/// Final tally of a migrate-apply run.
#[derive(Serialize, Clone, Default)]
struct MigrateSummary {
    /// Concrete operations attempted.
    total: usize,
    ok: usize,
    failed: usize,
    /// True if the run was cancelled partway (some actions may not have been attempted).
    cancelled: bool,
}

/// Apply migrate actions one at a time, reporting each concrete outcome and honouring cancellation.
/// Pure (no Tauri/IO of its own): `cancelled` is polled before each action (so a long single action —
/// e.g. a large directory copy — is not interrupted mid-way; cancellation granularity is per top-level
/// action), and `on_outcome` is called for every concrete outcome. Reuses the sync engine's `apply`.
fn run_migrate(
    left: &dyn SourceMut,
    right: &dyn SourceMut,
    actions: &[MigrateAction],
    cancelled: impl Fn() -> bool,
    mut on_outcome: impl FnMut(&ActionOutcome, MigrateReason),
) -> MigrateSummary {
    let mut summary = MigrateSummary::default();
    for action in actions {
        if cancelled() {
            summary.cancelled = true;
            break;
        }
        if !action.leaves.is_empty() {
            // Directory copy: apply leaf by leaf so cancellation fires between individual files.
            // Deletes and file actions fall through to the single-action path below.
            for leaf in &action.leaves {
                if cancelled() {
                    summary.cancelled = true;
                    return summary;
                }
                for outcome in sync_apply(left, right, &[leaf.to_sync_action(action.op)], false) {
                    summary.total += 1;
                    if outcome.ok { summary.ok += 1; } else { summary.failed += 1; }
                    on_outcome(&outcome, action.reason);
                }
            }
        } else {
            // File action, delete, or manually-constructed action (e.g. tests): single apply call.
            for outcome in sync_apply(left, right, &[action.to_sync_action()], false) {
                summary.total += 1;
                if outcome.ok { summary.ok += 1; } else { summary.failed += 1; }
                on_outcome(&outcome, action.reason);
            }
        }
    }
    summary
}

// ── M2: move semantics (delete origin after a verified-complete migration) ───────────────────────
//
// After the apply, if `delete_origin` is on, we re-compare origin↔destination with a FULL byte
// compare and delete the origin only if EVERY origin item is now verified identical in destination
// (minus exclusions). All-or-nothing: a partial origin-delete would let a later mirror pass see the
// deleted items as destination-only and remove them. The gate is derived purely from the
// re-comparison tree (`plan_origin_delete`), so its logic is unit-testable without Tauri.

/// An origin item the re-verification found NOT identical in destination — each one aborts the move.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginBlocker {
    rel_path: Vec<String>,
    /// Why it blocks (human-readable, for the summary).
    reason: &'static str,
}

/// What deleting the origin would entail, derived from a fresh full re-comparison after the apply.
#[derive(Debug, Default)]
struct OriginDeletePlan {
    /// Origin files verified identical in destination — safe to delete (one DeleteLeft each, for
    /// per-file progress + cancellation).
    files: Vec<RelPath>,
    /// Directories on the verified-identical paths, **bottom-up** — pruned only if empty at runtime,
    /// so any excluded survivors keep their parent directories alive.
    dirs: Vec<RelPath>,
    /// Origin items not verified identical in destination. Non-empty → the move is aborted, origin kept.
    blockers: Vec<OriginBlocker>,
}

/// Walk a fresh full re-comparison tree and decide what (if anything) can be deleted from the origin.
/// Exclusions appear in the tree as `Skipped` / `"filtered"` and are intentionally left alone (they do
/// not block the move and are never deleted) — this realises "delete the whole origin minus exclusions".
fn plan_origin_delete(root: &DiffEntry) -> OriginDeletePlan {
    let mut plan = OriginDeletePlan::default();
    for child in &root.children {
        visit_for_origin_delete(child, &mut plan);
    }
    // `dirs` are collected pre-order (parent before child); reverse so pruning runs deepest-first.
    plan.dirs.reverse();
    plan
}

/// `true` for an entry the user excluded via the filter (kept in origin, never a blocker).
fn is_filtered(entry: &DiffEntry) -> bool {
    entry.status == DiffStatus::Skipped && entry.detail.as_deref() == Some("filtered")
}

fn visit_for_origin_delete(entry: &DiffEntry, plan: &mut OriginDeletePlan) {
    match entry.status {
        // Verified identical: the whole subtree is deletable from origin.
        DiffStatus::Identical => collect_origin_leaves(entry, plan),
        // Destination-only: nothing in origin to delete; irrelevant to the move.
        DiffStatus::RightOnly => {}
        // Partially-divergent directory: some children identical, some not — recurse to find the
        // blockers (any blocker aborts the whole move, so the identical leaves collected here are moot).
        DiffStatus::Different if entry.is_dir => {
            for child in &entry.children {
                visit_for_origin_delete(child, plan);
            }
        }
        DiffStatus::Different => plan.blockers.push(blocker(entry, "differs in destination")),
        DiffStatus::LeftOnly => plan.blockers.push(blocker(entry, "only in origin (not copied)")),
        DiffStatus::Error => plan.blockers.push(blocker(entry, "comparison error")),
        // Excluded items are left alone; any other skip (symlink, non-descended) cannot be verified.
        DiffStatus::Skipped if is_filtered(entry) => {}
        DiffStatus::Skipped => plan.blockers.push(blocker(entry, "skipped (not verified)")),
    }
}

/// Collect every deletable leaf under a verified-identical entry: files to remove + directories to
/// prune. Excluded survivors (`Skipped`/`"filtered"`) inside an identical dir are left in place.
fn collect_origin_leaves(entry: &DiffEntry, plan: &mut OriginDeletePlan) {
    if entry.is_dir {
        plan.dirs.push(entry.rel_path.clone());
        for child in &entry.children {
            if is_filtered(child) {
                continue;
            }
            collect_origin_leaves(child, plan);
        }
    } else {
        plan.files.push(entry.rel_path.clone());
    }
}

fn blocker(entry: &DiffEntry, reason: &'static str) -> OriginBlocker {
    OriginBlocker {
        rel_path: entry.rel_path.components().to_vec(),
        reason,
    }
}

/// Final tally of the move (origin-delete) phase, carried alongside the apply summary in [`MIGRATE_DONE`].
#[derive(Serialize, Clone, Default)]
struct MoveSummary {
    /// True once the move was attempted (delete_origin on, apply clean). False → never reached the gate.
    attempted: bool,
    /// True if the origin was actually deleted (gate passed and the delete ran to completion).
    origin_deleted: bool,
    /// Origin files deleted.
    files_deleted: usize,
    /// Empty origin directories pruned.
    dirs_pruned: usize,
    /// Failures during the origin-delete itself.
    failed: usize,
    /// True if the origin-delete was cancelled partway.
    cancelled: bool,
    /// If the move did NOT proceed, the origin items that blocked it (capped for display).
    blockers: Vec<String>,
}

/// Remove one origin item, building an [`ActionOutcome`] (mirrors the sync engine's shape so progress
/// events are uniform). `remove` handles both files and empty directories.
fn left_remove(left: &dyn SourceMut, rel: &RelPath) -> ActionOutcome {
    let result = left.remove(rel);
    ActionOutcome {
        rel_path: rel.clone(),
        op: SyncOp::DeleteLeft,
        ok: result.is_ok(),
        error: result.err().map(|e| e.to_string()),
    }
}

/// Execute an [`OriginDeletePlan`] (caller must have checked `blockers` is empty): delete each verified
/// file, then prune directories that are empty at runtime (so excluded survivors keep their parents).
/// Cancellation is polled between operations.
fn run_origin_delete(
    left: &dyn SourceMut,
    plan: &OriginDeletePlan,
    cancelled: impl Fn() -> bool,
    mut on_delete: impl FnMut(&ActionOutcome),
) -> MoveSummary {
    let mut summary = MoveSummary { attempted: true, ..Default::default() };
    for rel in &plan.files {
        if cancelled() {
            summary.cancelled = true;
            return summary;
        }
        let outcome = left_remove(left, rel);
        if outcome.ok {
            summary.files_deleted += 1;
        } else {
            summary.failed += 1;
        }
        on_delete(&outcome);
    }
    for rel in &plan.dirs {
        if cancelled() {
            summary.cancelled = true;
            return summary;
        }
        // Prune only when empty: a directory still holding excluded survivors must stay.
        if !matches!(left.read_dir(rel), Ok(entries) if entries.is_empty()) {
            continue;
        }
        let outcome = left_remove(left, rel);
        if outcome.ok {
            summary.dirs_pruned += 1;
        } else {
            summary.failed += 1;
        }
        on_delete(&outcome);
    }
    summary.origin_deleted = !summary.cancelled && summary.failed == 0;
    summary
}

/// Re-verify (full byte compare) origin↔destination and, all-or-nothing, delete the origin if every
/// item is now verified identical (minus exclusions). Pure of Tauri: `on_delete` receives each removal
/// outcome for progress. Returns the [`MoveSummary`]; an `Err` is a hard re-comparison failure.
fn move_origin(
    left: &dyn SourceMut,
    right: &dyn SourceMut,
    filters: FilterSet,
    cancelled: impl Fn() -> bool,
    on_delete: impl FnMut(&ActionOutcome),
) -> Result<MoveSummary, String> {
    let cfg = CompareConfig {
        method: CompareMethod::Full,
        recursive: true,
        filters,
    };
    // Upcast the mutable sources to read-only for the comparison (SourceMut: Source).
    let lref: &dyn Source = left;
    let rref: &dyn Source = right;
    let report = engine_compare(lref, rref, &cfg).map_err(|e| e.to_string())?;
    let plan = plan_origin_delete(&report.root);
    if !plan.blockers.is_empty() {
        const MAX_SHOWN: usize = 20;
        let blockers = plan
            .blockers
            .iter()
            .take(MAX_SHOWN)
            .map(|b| format!("{} — {}", b.rel_path.join("/"), b.reason))
            .collect();
        return Ok(MoveSummary {
            attempted: true,
            blockers,
            ..Default::default()
        });
    }
    Ok(run_origin_delete(left, &plan, cancelled, on_delete))
}

/// Final tally of a migrate-apply run, emitted as [`MIGRATE_DONE`] when the background thread ends.
#[derive(Serialize, Clone)]
struct MigrateDone {
    generation: u64,
    summary: MigrateSummary,
    /// MOVE result (M2): present only when `delete_origin` was requested. `None` for a plain migration.
    #[serde(skip_serializing_if = "Option::is_none")]
    move_result: Option<MoveSummary>,
}

/// Signals which long phase of the apply is running, so the UI can show "Verifying…" / "Emptying
/// origin…" during the M2 re-verification (which emits no per-op progress until the origin-delete).
#[derive(Serialize, Clone)]
struct MigratePhase {
    generation: u64,
    /// `"verifying"` (full re-compare) or `"emptying_origin"` (deleting origin items).
    phase: &'static str,
}

/// Start a migration apply on a **background thread** and return immediately. Progress arrives via
/// [`MIGRATE_PROGRESS`] events (one per concrete operation); a final [`MIGRATE_DONE`] event carries
/// the tally. Spinning work off to a thread lets the webview process events in real time instead of
/// queueing them all until a blocking command returns — which would freeze the progress UI.
///
/// When `delete_origin` is set (M2 move semantics), a successful, uncancelled apply is followed by a
/// FULL re-comparison and — all-or-nothing — deletion of the origin (minus the `opts` exclusions). See
/// [`move_origin`].
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command args map 1:1 to the JS invoke payload.
fn migrate_apply(
    state: State<AppState>,
    app: AppHandle,
    left: SourceSpec,
    right: SourceSpec,
    actions: Vec<MigrateAction>,
    generation: u64,
    delete_origin: bool,
    opts: CompareOpts,
) -> Result<(), String> {
    // Generation 0 is the reserved cancel sentinel (see `migrate_cancel`): a real run must use a
    // non-zero, monotonically increasing generation so cancellation can reliably target it. Enforce it
    // here rather than trusting the caller, so a frontend bug surfaces loudly instead of silently
    // breaking cancellation.
    if generation == 0 {
        return Err("invalid migrate generation (0 is reserved for cancellation)".to_owned());
    }
    // For a move, build the exclusion filters up front so a bad pattern fails synchronously to the UI.
    let filters = if delete_origin {
        Some(FilterSet::new(&opts.include, &opts.exclude).map_err(|e| e.to_string())?)
    } else {
        None
    };
    // Build sources before claiming the generation: a connection error must propagate synchronously to
    // the UI without bumping the generation (which would cancel any apply still in flight from a prior run).
    let l = build_source_mut(&left)?;
    let r = build_source_mut(&right)?;
    state.migrate_gen.store(generation, Ordering::SeqCst);
    let cancel_token = Arc::clone(&state.migrate_gen);
    std::thread::spawn(move || {
        let is_cancelled = || cancel_token.load(Ordering::SeqCst) != generation;
        let emit_progress = |outcome: &ActionOutcome, reason: MigrateReason| {
            let _ = app.emit(
                MIGRATE_PROGRESS,
                MigrateProgress {
                    generation,
                    rel_path: outcome.rel_path.components().to_vec(),
                    op: outcome.op,
                    reason,
                    ok: outcome.ok,
                    error: outcome.error.clone(),
                },
            );
        };
        let summary = run_migrate(l.as_ref(), r.as_ref(), &actions, is_cancelled, |o, reason| {
            emit_progress(o, reason)
        });

        // M2 move: re-verify (full) and, all-or-nothing, delete the origin. Only when requested and the
        // apply completed cleanly — any apply failure or cancellation keeps the origin untouched.
        let move_result = match filters {
            Some(_) if summary.cancelled => Some(MoveSummary::default()),
            Some(_) if summary.failed > 0 => Some(MoveSummary {
                attempted: true,
                blockers: vec![format!(
                    "{} item(s) failed to copy — origin kept",
                    summary.failed
                )],
                ..Default::default()
            }),
            Some(filters) => {
                let _ = app.emit(MIGRATE_PHASE, MigratePhase { generation, phase: "verifying" });
                let mut emitted_emptying = false;
                let result = move_origin(l.as_ref(), r.as_ref(), filters, is_cancelled, |outcome| {
                    if !emitted_emptying {
                        emitted_emptying = true;
                        let _ = app.emit(
                            MIGRATE_PHASE,
                            MigratePhase { generation, phase: "emptying_origin" },
                        );
                    }
                    emit_progress(outcome, MigrateReason::Moved);
                });
                Some(result.unwrap_or_else(|e| MoveSummary {
                    attempted: true,
                    blockers: vec![format!("re-verification failed: {e}")],
                    ..Default::default()
                }))
            }
            None => None,
        };

        let _ = app.emit(MIGRATE_DONE, MigrateDone { generation, summary, move_result });
    });
    Ok(())
}

/// Cancel the migrate-apply in progress (if any): sets the generation to 0 (a value no run uses), so
/// the running loop stops before its next action.
#[tauri::command]
fn migrate_cancel(state: State<AppState>) {
    state.migrate_gen.store(0, Ordering::SeqCst);
}

/// Preview (dry-run) the effect of a batch of sync actions — writes nothing.
#[tauri::command]
fn plan_actions(
    left: SourceSpec,
    right: SourceSpec,
    actions: Vec<SyncAction>,
) -> Result<Vec<ActionOutcome>, String> {
    let l = build_source_mut(&left)?;
    let r = build_source_mut(&right)?;
    Ok(sync_apply(l.as_ref(), r.as_ref(), &actions, true))
}

/// Execute a batch of sync actions (copy/delete left↔right) and report per-operation outcomes.
#[tauri::command]
fn apply_actions(
    left: SourceSpec,
    right: SourceSpec,
    actions: Vec<SyncAction>,
) -> Result<Vec<ActionOutcome>, String> {
    let l = build_source_mut(&left)?;
    let r = build_source_mut(&right)?;
    Ok(sync_apply(l.as_ref(), r.as_ref(), &actions, false))
}

/// Result of diffing two files: a text line/word diff, a binary byte-equality verdict, or a signal
/// that the file exceeds `TEXT_CAP` and should be opened via the large-file flow (`diff_file_large`).
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DiffResult {
    Text { diff: FileDiff },
    Binary { identical: bool },
    TooLarge { left_size: u64, right_size: u64 },
}

/// Per-side metadata captured at read time: a content fingerprint (for conflict detection) plus the
/// line-ending style and whether the file ends with a newline — so a save round-trips byte-for-byte
/// (no lost trailing `\n`, no silent CRLF→LF) and an unchanged side stays equal on disk.
#[derive(Serialize)]
struct FileMeta {
    fp: String,
    eol: String,
    final_nl: bool,
    /// Modified / creation time (epoch ms) for the side-by-side header; `None` when unavailable.
    mtime: Option<i64>,
    created: Option<i64>,
}

/// `diff_file` payload: the diff plus per-side metadata (fingerprint + EOL/final-newline).
#[derive(Serialize)]
struct DiffFileResult {
    result: DiffResult,
    left: FileMeta,
    right: FileMeta,
}

/// A file's line ending for save reconstruction — CRLF if it contains any `\r\n`, else LF. (Mixed-EOL
/// files are normalised to CRLF on save; acceptable for our use.)
fn detect_eol(s: &str) -> &'static str {
    if s.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Read a file from an already-built source, treating a missing file as empty bytes (same contract
/// as the old `read_side` helper). The caller keeps ownership of the `Source` so it stays cached.
fn read_side_cached(
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
fn diff_file(state: State<AppState>, left: FileRef, right: FileRef) -> Result<DiffFileResult, String> {
    let src_l = cached_source(&state, &left.source)?;
    let src_r = cached_source(&state, &right.source)?;
    diff_file_inner(src_l.as_ref(), &rel_from_str(&left.rel), src_r.as_ref(), &rel_from_str(&right.rel))
}

/// Core of [`diff_file`], over already-built sources (so it's unit-testable without Tauri `State`).
fn diff_file_inner(src_l: &dyn Source, rel_l: &RelPath, src_r: &dyn Source, rel_r: &RelPath) -> Result<DiffFileResult, String> {
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
enum LargeDiffResult {
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
fn diff_file_large(
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
fn fingerprint(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Re-diff two in-memory texts (used by the side-by-side after a copy-change/edit).
#[tauri::command]
fn diff_strings(left: String, right: String) -> FileDiff {
    diff_text(&left, &right)
}

/// Outcome of a save: written (with the new fingerprint) or refused because the file changed on disk.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SaveResult {
    Saved { fp: String },
    Conflict,
}

/// Write `contents` to `path` (save a merged side). Unless `force`, refuses with `Conflict` when the
/// file's current fingerprint differs from `expect` (i.e. it changed on disk since the UI read it).
#[tauri::command]
fn save_file(
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
const HEX_CAP: usize = 256 * 1024;

/// Bytes of two files for a hex compare. Each side is capped at `HEX_CAP`; `*_len` are the true lengths.
#[derive(Serialize)]
struct HexCompare {
    left: Vec<u8>,
    right: Vec<u8>,
    left_len: u64,
    right_len: u64,
    truncated: bool,
}

/// Read two files' first `HEX_CAP` bytes for a side-by-side hex comparison; `*_len` are the true sizes,
/// `truncated` if either exceeds the cap. Works for files of any size (prefix only — never rejects).
/// Uses the cached source so a binary opened from a folder compare reuses the existing connection.
#[tauri::command]
fn hex_compare(state: State<AppState>, left: FileRef, right: FileRef) -> Result<HexCompare, String> {
    let src_l = cached_source(&state, &left.source)?;
    let src_r = cached_source(&state, &right.source)?;
    Ok(hex_compare_inner(src_l.as_ref(), &rel_from_str(&left.rel), src_r.as_ref(), &rel_from_str(&right.rel)))
}

/// Core of [`hex_compare`], over already-built sources (unit-testable). Reads each side's first
/// `HEX_CAP` bytes (prefix only — never rejects a large file); `*_len` are the true sizes.
fn hex_compare_inner(src_l: &dyn Source, rel_l: &RelPath, src_r: &dyn Source, rel_r: &RelPath) -> HexCompare {
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
const IMAGE_CAP: u64 = 25_000_000;

/// Read a file's raw bytes (for blob-URL image loading — same-origin, so the canvas isn't tainted and
/// pixel-diff works). Returned as an ArrayBuffer to the UI. Size-guarded (images can be larger than text).
#[tauri::command]
fn read_bytes(file: FileRef) -> Result<tauri::ipc::Response, String> {
    let bytes = read_capped(&file, IMAGE_CAP)?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// One configurable field of a source type — the UI renders a config form from these.
#[derive(Serialize)]
struct FieldSpec {
    /// Config key. Dotted (`auth.password`) for nested fields (the UI assembles the `SourceSpec`).
    key: &'static str,
    label: &'static str,
    /// Widget hint: `text` | `number` | `password` | `path` | `textarea` | `select`.
    kind: &'static str,
    required: bool,
    /// Secret material — the UI must mask it and avoid persisting it in plaintext.
    secret: bool,
    /// Default value, if any (e.g. SFTP port `22`).
    default: Option<&'static str>,
    /// Allowed values for a `select` field.
    options: Vec<&'static str>,
    /// Conditional display `"key=value"` — show only when another field has this value.
    show_when: Option<&'static str>,
}

impl FieldSpec {
    fn new(key: &'static str, label: &'static str, kind: &'static str, required: bool) -> Self {
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
    fn secret(mut self) -> Self {
        self.secret = true;
        self
    }
    fn default(mut self, d: &'static str) -> Self {
        self.default = Some(d);
        self
    }
    fn options(mut self, o: &[&'static str]) -> Self {
        self.options = o.to_vec();
        self
    }
    fn show_when(mut self, w: &'static str) -> Self {
        self.show_when = Some(w);
        self
    }
}

/// A source type the UI can offer: id, display name, icon, capabilities, and its config form.
#[derive(Serialize)]
struct SourceTypeInfo {
    id: &'static str,
    name: &'static str,
    /// Icon for the picker (emoji today) — sourced from the `SourceKind`, so the frontend needn't switch.
    icon: &'static str,
    /// What the type can do — gates which actions the UI offers. See `confold_vfs::Capabilities`.
    capabilities: Capabilities,
    fields: Vec<FieldSpec>,
}

/// Catalog of source types the UI can offer (drives the type picker + per-type config form), built from
/// the `SourceKind` registry. A new backend appears here automatically once registered.
#[tauri::command]
fn source_types() -> Vec<SourceTypeInfo> {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            compare,
            compare_level,
            migrate_actions,
            sync_actions,
            migrate_apply,
            migrate_cancel,
            plan_actions,
            apply_actions,
            diff_file,
            diff_file_large,
            diff_strings,
            save_file,
            hex_compare,
            read_bytes,
            source_types,
            test_source
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// An `fs` `SourceSpec` rooted at `root` (test helper).
    fn fs_spec(root: String) -> SourceSpec {
        SourceSpec {
            kind: "fs".to_string(),
            fields: BTreeMap::from([("root".to_string(), root)]),
        }
    }

    /// A `FileRef` for a file named `name` in directory `dir` (an `fs` source rooted at `dir`).
    fn fref(dir: &Path, name: &str) -> FileRef {
        FileRef {
            source: fs_spec(dir.to_str().unwrap().to_string()),
            rel: name.to_string(),
        }
    }

    #[test]
    fn migrate_actions_collects_per_flags() {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        // only in origin (a file and a whole subtree)
        fs::write(l.path().join("new.txt"), b"x").unwrap();
        fs::create_dir(l.path().join("newdir")).unwrap();
        fs::write(l.path().join("newdir").join("a.txt"), b"a").unwrap();
        // different content on both sides
        fs::write(l.path().join("diff.txt"), b"left").unwrap();
        fs::write(r.path().join("diff.txt"), b"right!").unwrap();
        // only in destination
        fs::write(r.path().join("extra.txt"), b"e").unwrap();
        // identical
        fs::write(l.path().join("same.txt"), b"s").unwrap();
        fs::write(r.path().join("same.txt"), b"s").unwrap();

        let cfg = CompareConfig {
            method: CompareMethod::Full,
            recursive: true,
            filters: FilterSet::default(),
        };
        let report =
            engine_compare(&LocalSource::new(l.path()), &LocalSource::new(r.path()), &cfg).unwrap();

        let key = |a: &MigrateAction| a.rel_path.join("/");
        let collect = |flags: MigrateFlags| {
            let mut v = Vec::new();
            collect_migrate(&report.root, flags, &mut v);
            v
        };

        // All categories enabled.
        let all = collect(MigrateFlags {
            copy_new: true,
            overwrite_different: true,
            delete_extra: true,
        });
        let mut got: Vec<(String, MigrateReason, bool)> =
            all.iter().map(|a| (key(a), a.reason, a.is_dir)).collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            vec![
                ("diff.txt".to_string(), MigrateReason::Different, false),
                ("extra.txt".to_string(), MigrateReason::Extra, false),
                ("new.txt".to_string(), MigrateReason::New, false),
                ("newdir".to_string(), MigrateReason::New, true),
            ]
        );
        // The new subtree is ONE recursive copy action — its child is not enumerated separately.
        assert!(!all.iter().any(|a| key(a) == "newdir/a.txt"));
        // Identical items are never touched.
        assert!(!all.iter().any(|a| key(a) == "same.txt"));

        // copy_new only → just the origin-only items.
        let copy_only = collect(MigrateFlags {
            copy_new: true,
            overwrite_different: false,
            delete_extra: false,
        });
        let mut paths: Vec<String> = copy_only.iter().map(|a| key(a)).collect();
        paths.sort();
        assert_eq!(paths, vec!["new.txt", "newdir"]);

        // delete_extra only → just the destination-only item, as a delete-on-right.
        let del_only = collect(MigrateFlags {
            copy_new: false,
            overwrite_different: false,
            delete_extra: true,
        });
        assert_eq!(del_only.len(), 1);
        assert_eq!(key(&del_only[0]), "extra.txt");
        assert_eq!(del_only[0].reason, MigrateReason::Extra);

        // Everything off → no actions.
        assert!(collect(MigrateFlags {
            copy_new: false,
            overwrite_different: false,
            delete_extra: false,
        })
        .is_empty());
    }

    #[test]
    fn run_migrate_applies_reports_and_cancels() {
        use std::cell::Cell;

        // Origin with two files and a subtree; destination empty.
        let l = tempfile::tempdir().unwrap();
        let actions = vec![
            MigrateAction { rel_path: vec!["a.txt".to_string()], op: SyncOp::CopyLeftToRight, is_dir: false, reason: MigrateReason::New, item_count: 1, leaves: vec![] },
            MigrateAction { rel_path: vec!["b.txt".to_string()], op: SyncOp::CopyLeftToRight, is_dir: false, reason: MigrateReason::New, item_count: 1, leaves: vec![] },
            // Directory with pre-expanded leaves (the c.txt file inside).
            MigrateAction { rel_path: vec!["d".to_string()], op: SyncOp::CopyLeftToRight, is_dir: true, reason: MigrateReason::New, item_count: 2,
                leaves: vec![LeafOp { rel_path: vec!["d".to_string(), "c.txt".to_string()], is_dir: false }] },
        ];
        fs::write(l.path().join("a.txt"), b"A").unwrap();
        fs::write(l.path().join("b.txt"), b"B").unwrap();
        fs::create_dir(l.path().join("d")).unwrap();
        fs::write(l.path().join("d").join("c.txt"), b"C").unwrap();
        let left = LocalSource::new(l.path());

        // No cancellation: everything applies; the directory action expands into per-descendant outcomes.
        let r = tempfile::tempdir().unwrap();
        let right = LocalSource::new(r.path());
        let mut seen: Vec<(String, bool)> = Vec::new();
        let summary = run_migrate(&left, &right, &actions, || false, |o, _reason| {
            seen.push((o.rel_path.components().join("/"), o.ok))
        });
        assert!(!summary.cancelled);
        assert_eq!(summary.failed, 0);
        assert_eq!(fs::read(r.path().join("a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(r.path().join("d").join("c.txt")).unwrap(), b"C");
        // Per-file granularity: the directory's child shows up as its own outcome.
        assert!(seen.iter().any(|(p, ok)| p == "d/c.txt" && *ok));

        // Cancel after the first action is applied: only a.txt lands; b.txt and d/ are skipped.
        let r2 = tempfile::tempdir().unwrap();
        let right2 = LocalSource::new(r2.path());
        let applied = Cell::new(0usize);
        let summary2 = run_migrate(
            &left,
            &right2,
            &actions,
            || applied.get() >= 1, // polled BEFORE each action
            |_, _reason| applied.set(applied.get() + 1),
        );
        assert!(summary2.cancelled);
        assert!(r2.path().join("a.txt").exists());
        assert!(!r2.path().join("b.txt").exists());
        assert!(!r2.path().join("d").exists());
    }

    #[test]
    fn run_migrate_reports_per_item_failure_and_keeps_going() {
        // The whole point of M1's per-item apply: one action failing must NOT abort the run, must be
        // tallied as `failed`, and must surface its error to the progress callback — while the other
        // actions still apply. We force a deterministic failure with a copy whose source is missing
        // (open fails → ok:false), portable across platforms (no permission/chmod tricks).
        let l = tempfile::tempdir().unwrap();
        fs::write(l.path().join("ok1.txt"), b"1").unwrap();
        fs::write(l.path().join("ok2.txt"), b"2").unwrap();
        // NB: "ghost.txt" is intentionally NOT created on the left.
        let left = LocalSource::new(l.path());
        let r = tempfile::tempdir().unwrap();
        let right = LocalSource::new(r.path());

        let new = |name: &str| MigrateAction {
            rel_path: vec![name.to_string()],
            op: SyncOp::CopyLeftToRight,
            is_dir: false,
            reason: MigrateReason::New,
            item_count: 1,
            leaves: vec![],
        };
        // Failure is in the MIDDLE, so we also prove the run continues past it.
        let actions = vec![new("ok1.txt"), new("ghost.txt"), new("ok2.txt")];

        let mut outcomes: Vec<(String, bool, Option<String>)> = Vec::new();
        let summary = run_migrate(&left, &right, &actions, || false, |o, _reason| {
            outcomes.push((
                o.rel_path.components().join("/"),
                o.ok,
                o.error.clone(),
            ))
        });

        // Tally: three attempted, two ok, one failed, not cancelled.
        assert!(!summary.cancelled);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.ok, 2);
        assert_eq!(summary.failed, 1);

        // The failed item is reported with ok:false AND a non-empty error string.
        let ghost = outcomes.iter().find(|(p, ..)| p == "ghost.txt").unwrap();
        assert!(!ghost.1, "ghost.txt should have failed");
        assert!(ghost.2.as_deref().is_some_and(|e| !e.is_empty()), "expected an error message");

        // The run kept going: the action AFTER the failure still applied to the destination.
        assert_eq!(fs::read(r.path().join("ok1.txt")).unwrap(), b"1");
        assert_eq!(fs::read(r.path().join("ok2.txt")).unwrap(), b"2");
        assert!(!r.path().join("ghost.txt").exists());
    }

    #[test]
    fn run_migrate_cancels_within_directory_copy() {
        // A single directory action with two files inside. Cancelling after the first leaf means the
        // second file is NOT copied — per-file cancellation works inside a directory.
        let l = tempfile::tempdir().unwrap();
        fs::create_dir(l.path().join("d")).unwrap();
        fs::write(l.path().join("d").join("f1.txt"), b"F1").unwrap();
        fs::write(l.path().join("d").join("f2.txt"), b"F2").unwrap();
        let left = LocalSource::new(l.path());
        let r = tempfile::tempdir().unwrap();
        let right = LocalSource::new(r.path());

        let actions = vec![MigrateAction {
            rel_path: vec!["d".to_string()],
            op: SyncOp::CopyLeftToRight,
            is_dir: true,
            reason: MigrateReason::New,
            item_count: 2,
            leaves: vec![
                LeafOp { rel_path: vec!["d".to_string(), "f1.txt".to_string()], is_dir: false },
                LeafOp { rel_path: vec!["d".to_string(), "f2.txt".to_string()], is_dir: false },
            ],
        }];

        let applied = std::cell::Cell::new(0usize);
        let summary = run_migrate(
            &left,
            &right,
            &actions,
            || applied.get() >= 1, // cancel after the first leaf
            |_, _| applied.set(applied.get() + 1),
        );
        assert!(summary.cancelled);
        // First leaf landed; second was skipped.
        assert!(r.path().join("d").join("f1.txt").exists());
        assert!(!r.path().join("d").join("f2.txt").exists());
    }

    #[test]
    fn run_migrate_cancels_within_directory_delete() {
        // A single delete action with two files. Cancelling after the first means the second survives.
        let r = tempfile::tempdir().unwrap();
        fs::create_dir(r.path().join("d")).unwrap();
        fs::write(r.path().join("d").join("f1.txt"), b"F1").unwrap();
        fs::write(r.path().join("d").join("f2.txt"), b"F2").unwrap();
        let left  = LocalSource::new(r.path()); // unused for delete but required by signature
        let right = LocalSource::new(r.path());

        let actions = vec![MigrateAction {
            rel_path: vec!["d".to_string()],
            op: SyncOp::DeleteRight,
            is_dir: true,
            reason: MigrateReason::Extra,
            item_count: 2,
            leaves: vec![
                LeafOp { rel_path: vec!["d".to_string(), "f1.txt".to_string()], is_dir: false },
                LeafOp { rel_path: vec!["d".to_string(), "f2.txt".to_string()], is_dir: false },
                LeafOp { rel_path: vec!["d".to_string()], is_dir: true },
            ],
        }];

        let applied = std::cell::Cell::new(0usize);
        let summary = run_migrate(
            &left, &right, &actions,
            || applied.get() >= 1,
            |_, _| applied.set(applied.get() + 1),
        );
        assert!(summary.cancelled);
        assert!(!r.path().join("d").join("f1.txt").exists()); // first file deleted
        assert!(r.path().join("d").join("f2.txt").exists());  // second survived
        assert!(r.path().join("d").exists());                 // dir still there (not yet deleted)
    }

    // ── M2: move semantics (delete origin after verified-complete migration) ─────────────────────

    /// Full compare of two `fs` roots with the given excludes — mimics the M2 re-verification pass.
    fn reverify(left: &Path, right: &Path, exclude: &[&str]) -> DiffReport {
        let exclude: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
        let cfg = CompareConfig {
            method: CompareMethod::Full,
            recursive: true,
            filters: FilterSet::new(&[], &exclude).unwrap(),
        };
        engine_compare(&LocalSource::new(left), &LocalSource::new(right), &cfg).unwrap()
    }

    #[test]
    fn move_empties_origin_when_all_verified_identical() {
        // Post-apply state: origin and destination are byte-identical (files + a subtree). The move
        // must delete every origin file AND prune the now-empty directory, leaving the origin empty.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        for root in [l.path(), r.path()] {
            fs::write(root.join("a.txt"), b"A").unwrap();
            fs::write(root.join("b.txt"), b"B").unwrap();
            fs::create_dir(root.join("d")).unwrap();
            fs::write(root.join("d").join("c.txt"), b"C").unwrap();
        }
        let left = LocalSource::new(l.path());
        let right = LocalSource::new(r.path());

        let mut deleted: Vec<String> = Vec::new();
        let summary = move_origin(
            &left,
            &right,
            FilterSet::default(),
            || false,
            |o| {
                assert!(o.ok, "delete failed: {:?}", o.error);
                deleted.push(o.rel_path.components().join("/"));
            },
        )
        .unwrap();

        assert!(summary.origin_deleted, "origin should have been deleted");
        assert!(summary.blockers.is_empty());
        assert_eq!(summary.files_deleted, 3); // a.txt, b.txt, d/c.txt
        assert_eq!(summary.dirs_pruned, 1); // d
        assert_eq!(summary.failed, 0);
        // The whole origin is gone; the destination is untouched.
        assert_eq!(fs::read_dir(l.path()).unwrap().count(), 0, "origin not empty");
        assert!(r.path().join("a.txt").exists() && r.path().join("d").join("c.txt").exists());
        assert!(deleted.contains(&"d/c.txt".to_string()) && deleted.contains(&"d".to_string()));
    }

    #[test]
    fn move_keeps_whole_origin_when_one_item_not_identical() {
        // All-or-nothing: a single non-identical origin item (here, one only-in-origin file — e.g. its
        // copy was skipped or failed) blocks the move. NOTHING is deleted from the origin.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        fs::write(l.path().join("ok.txt"), b"same").unwrap();
        fs::write(r.path().join("ok.txt"), b"same").unwrap();
        fs::write(l.path().join("orphan.txt"), b"only in origin").unwrap(); // not in destination
        let left = LocalSource::new(l.path());
        let right = LocalSource::new(r.path());

        let summary = move_origin(&left, &right, FilterSet::default(), || false, |_| {
            panic!("nothing must be deleted when the move is blocked");
        })
        .unwrap();

        assert!(!summary.origin_deleted);
        assert!(summary.attempted);
        assert_eq!(summary.files_deleted, 0);
        assert_eq!(summary.blockers.len(), 1);
        assert!(summary.blockers[0].contains("orphan.txt"));
        assert!(summary.blockers[0].contains("only in origin"));
        // Both origin files survive — even the one that WAS identical.
        assert!(l.path().join("ok.txt").exists() && l.path().join("orphan.txt").exists());
    }

    #[test]
    fn move_leaves_excluded_files_in_origin() {
        // "Delete the whole origin minus exclusions": an excluded file does not block the move, is not
        // deleted, and its parent directory (now holding only the survivor) is NOT pruned.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        // Top-level identical file → deleted.
        fs::write(l.path().join("move.txt"), b"M").unwrap();
        fs::write(r.path().join("move.txt"), b"M").unwrap();
        // A directory with one identical file (deleted) and one excluded log (kept → dir survives).
        for root in [l.path(), r.path()] {
            fs::create_dir(root.join("d")).unwrap();
            fs::write(root.join("d").join("data.bin"), b"D").unwrap();
        }
        fs::write(l.path().join("d").join("keep.log"), b"L").unwrap(); // only in origin, excluded
        let left = LocalSource::new(l.path());
        let right = LocalSource::new(r.path());

        let summary =
            move_origin(&left, &right, FilterSet::new(&[], &["*.log".to_string()]).unwrap(), || false, |o| {
                assert!(o.ok, "delete failed: {:?}", o.error);
            })
            .unwrap();

        assert!(summary.origin_deleted, "excluded file must not block the move");
        assert!(summary.blockers.is_empty());
        assert_eq!(summary.files_deleted, 2); // move.txt + d/data.bin
        assert_eq!(summary.dirs_pruned, 0); // d kept (still holds keep.log)
        assert!(!l.path().join("move.txt").exists());
        assert!(!l.path().join("d").join("data.bin").exists());
        assert!(l.path().join("d").join("keep.log").exists(), "excluded file must survive");
        assert!(l.path().join("d").exists(), "dir with a survivor must not be pruned");
    }

    #[test]
    fn move_cancellation_leaves_origin_untouched() {
        // Cancelling before the first removal: the re-compare ran, but no origin item is deleted.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        fs::write(l.path().join("a.txt"), b"A").unwrap();
        fs::write(r.path().join("a.txt"), b"A").unwrap();
        let left = LocalSource::new(l.path());
        let right = LocalSource::new(r.path());

        let summary = move_origin(&left, &right, FilterSet::default(), || true, |_| {
            panic!("nothing must be deleted once cancelled");
        })
        .unwrap();

        assert!(summary.cancelled);
        assert!(!summary.origin_deleted);
        assert_eq!(summary.files_deleted, 0);
        assert!(l.path().join("a.txt").exists());
    }

    #[test]
    fn plan_origin_delete_classifies_each_status() {
        // Drive the pure gate from a real re-comparison covering every category at once.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        // identical file + identical subtree → deletable
        fs::write(l.path().join("same.txt"), b"s").unwrap();
        fs::write(r.path().join("same.txt"), b"s").unwrap();
        for root in [l.path(), r.path()] {
            fs::create_dir(root.join("kept")).unwrap();
            fs::write(root.join("kept").join("inner.txt"), b"i").unwrap();
        }
        // different content → blocker
        fs::write(l.path().join("diff.txt"), b"left").unwrap();
        fs::write(r.path().join("diff.txt"), b"right").unwrap();
        // only in origin → blocker
        fs::write(l.path().join("orphan.txt"), b"o").unwrap();
        // only in destination → ignored (no origin to delete)
        fs::write(r.path().join("extra.txt"), b"e").unwrap();
        // excluded → neither deleted nor a blocker
        fs::write(l.path().join("skip.log"), b"x").unwrap();

        let report = reverify(l.path(), r.path(), &["*.log"]);
        let plan = plan_origin_delete(&report.root);

        let files: Vec<String> = plan.files.iter().map(|p| p.components().join("/")).collect();
        let dirs: Vec<String> = plan.dirs.iter().map(|p| p.components().join("/")).collect();
        let blockers: Vec<(String, &str)> =
            plan.blockers.iter().map(|b| (b.rel_path.join("/"), b.reason)).collect();

        assert!(files.contains(&"same.txt".to_string()));
        assert!(files.contains(&"kept/inner.txt".to_string()));
        assert!(!files.iter().any(|f| f == "skip.log"), "excluded file must not be deletable");
        assert_eq!(dirs, vec!["kept".to_string()]);
        assert_eq!(blockers.len(), 2);
        assert!(blockers.iter().any(|(p, why)| p == "diff.txt" && *why == "differs in destination"));
        assert!(blockers.iter().any(|(p, why)| p == "orphan.txt" && *why == "only in origin (not copied)"));
    }

    // ── Sync (S1): bidirectional reconciliation ─────────────────────────────────────────────────

    /// Compare two `fs` roots (full, recursive) and collect sync actions for `flags`.
    fn sync_plan(left: &Path, right: &Path, flags: SyncFlags) -> Vec<MigrateAction> {
        let cfg = CompareConfig {
            method: CompareMethod::Full,
            recursive: true,
            filters: FilterSet::default(),
        };
        let report =
            engine_compare(&LocalSource::new(left), &LocalSource::new(right), &cfg).unwrap();
        let mut actions = Vec::new();
        collect_sync(&report.root, flags, &mut actions);
        actions
    }

    /// A common fixture: a left-only file+dir, a right-only file+dir, a conflict, and an identical file.
    fn sync_fixture() -> (tempfile::TempDir, tempfile::TempDir) {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        fs::write(l.path().join("new_left.txt"), b"L").unwrap();
        fs::create_dir(l.path().join("ldir")).unwrap();
        fs::write(l.path().join("ldir").join("a.txt"), b"a").unwrap();
        fs::write(r.path().join("new_right.txt"), b"R").unwrap();
        fs::create_dir(r.path().join("rdir")).unwrap();
        fs::write(r.path().join("rdir").join("b.txt"), b"b").unwrap();
        // Conflict: present on both, different content (left larger, for the Larger rule).
        fs::write(l.path().join("conflict.txt"), b"left-is-bigger").unwrap();
        fs::write(r.path().join("conflict.txt"), b"r").unwrap();
        // Identical: never touched.
        fs::write(l.path().join("same.txt"), b"s").unwrap();
        fs::write(r.path().join("same.txt"), b"s").unwrap();
        (l, r)
    }

    /// (path, op) pairs, sorted — the shape we assert against.
    fn ops(actions: &[MigrateAction]) -> Vec<(String, SyncOp)> {
        let mut v: Vec<(String, SyncOp)> =
            actions.iter().map(|a| (a.rel_path.join("/"), a.op)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    #[test]
    fn collect_sync_trust_left_only_is_migrate_left_to_right() {
        let (l, r) = sync_fixture();
        let flags = SyncFlags {
            trust_left: true,
            trust_right: false,
            delete_diffs: false,
            conflict_rule: ConflictRule::Newer,
        };
        // Left is the sole authority: its unique items + the conflict flow right; right-only items are
        // left untouched (delete off); the conflict is won by left.
        assert_eq!(
            ops(&sync_plan(l.path(), r.path(), flags)),
            vec![
                ("conflict.txt".to_string(), SyncOp::CopyLeftToRight),
                ("ldir".to_string(), SyncOp::CopyLeftToRight),
                ("new_left.txt".to_string(), SyncOp::CopyLeftToRight),
            ]
        );
    }

    #[test]
    fn collect_sync_trust_left_with_delete_removes_right_extras() {
        let (l, r) = sync_fixture();
        let flags = SyncFlags {
            trust_left: true,
            trust_right: false,
            delete_diffs: true,
            conflict_rule: ConflictRule::Newer,
        };
        // Same as above plus the untrusted side's (right) extras are deleted.
        assert_eq!(
            ops(&sync_plan(l.path(), r.path(), flags)),
            vec![
                ("conflict.txt".to_string(), SyncOp::CopyLeftToRight),
                ("ldir".to_string(), SyncOp::CopyLeftToRight),
                ("new_left.txt".to_string(), SyncOp::CopyLeftToRight),
                ("new_right.txt".to_string(), SyncOp::DeleteRight),
                ("rdir".to_string(), SyncOp::DeleteRight),
            ]
        );
    }

    #[test]
    fn collect_sync_trust_right_only_mirrors_to_left() {
        let (l, r) = sync_fixture();
        let flags = SyncFlags {
            trust_left: false,
            trust_right: true,
            delete_diffs: true,
            conflict_rule: ConflictRule::Newer,
        };
        // Right is the sole authority: its items flow left, the conflict is won by right, and left's
        // extras are deleted.
        assert_eq!(
            ops(&sync_plan(l.path(), r.path(), flags)),
            vec![
                ("conflict.txt".to_string(), SyncOp::CopyRightToLeft),
                ("ldir".to_string(), SyncOp::DeleteLeft),
                ("new_left.txt".to_string(), SyncOp::DeleteLeft),
                ("new_right.txt".to_string(), SyncOp::CopyRightToLeft),
                ("rdir".to_string(), SyncOp::CopyRightToLeft),
            ]
        );
    }

    #[test]
    fn collect_sync_trust_both_is_union_with_conflict_rule() {
        let (l, r) = sync_fixture();
        // Larger wins → the conflict (left is bigger) flows left→right. delete_diffs is irrelevant when
        // both are trusted (every one-sided item is copied, never deleted).
        let larger = SyncFlags {
            trust_left: true,
            trust_right: true,
            delete_diffs: true,
            conflict_rule: ConflictRule::Larger,
        };
        assert_eq!(
            ops(&sync_plan(l.path(), r.path(), larger)),
            vec![
                ("conflict.txt".to_string(), SyncOp::CopyLeftToRight),
                ("ldir".to_string(), SyncOp::CopyLeftToRight),
                ("new_left.txt".to_string(), SyncOp::CopyLeftToRight),
                ("new_right.txt".to_string(), SyncOp::CopyRightToLeft),
                ("rdir".to_string(), SyncOp::CopyRightToLeft),
            ]
        );

        // Manual → the conflict is omitted (left for the user to resolve in Compare); copies still flow.
        let manual = SyncFlags { conflict_rule: ConflictRule::Manual, ..larger };
        let plan = sync_plan(l.path(), r.path(), manual);
        assert!(!plan.iter().any(|a| a.rel_path.join("/") == "conflict.txt"));
        assert_eq!(plan.len(), 4); // the four one-sided copies, no conflict
    }

    #[test]
    fn resolve_conflict_honours_rules_and_leaves_ties_manual() {
        let entry = |size_l: u64, mtime_l: Option<i64>, size_r: u64, mtime_r: Option<i64>| {
            let meta = |size, mtime| EntryMeta {
                name: "f".into(),
                rel_path: RelPath::root().child("f"),
                kind: confold_core::EntryKind::File,
                size,
                mtime,
                created: None,
            };
            DiffEntry {
                rel_path: RelPath::root().child("f"),
                name: "f".into(),
                is_dir: false,
                status: DiffStatus::Different,
                left: Some(meta(size_l, mtime_l)),
                right: Some(meta(size_r, mtime_r)),
                detail: None,
                children: vec![],
            }
        };
        let both = |rule| SyncFlags {
            trust_left: true,
            trust_right: true,
            delete_diffs: false,
            conflict_rule: rule,
        };

        // Newer wins by mtime; equal/unknown mtime → manual (None).
        let e = entry(1, Some(200), 1, Some(100));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Newer)), Some(SyncOp::CopyLeftToRight));
        let e = entry(1, Some(100), 1, Some(200));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Newer)), Some(SyncOp::CopyRightToLeft));
        let e = entry(1, Some(100), 1, Some(100));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Newer)), None); // tie
        let e = entry(1, None, 1, Some(100));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Newer)), None); // unknown

        // Larger wins by size; equal size → manual.
        let e = entry(10, None, 5, None);
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Larger)), Some(SyncOp::CopyLeftToRight));
        let e = entry(5, None, 5, None);
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Larger)), None); // tie

        // Manual never auto-resolves, even with a clear winner.
        let e = entry(10, Some(200), 5, Some(100));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Manual)), None);

        // One-sided trust: the trusted side always wins, regardless of metadata or rule.
        let e = entry(1, Some(100), 999, Some(999));
        let left_only = SyncFlags { trust_right: false, ..both(ConflictRule::Newer) };
        assert_eq!(resolve_conflict(&e, left_only), Some(SyncOp::CopyLeftToRight));
        let right_only = SyncFlags { trust_left: false, ..both(ConflictRule::Newer) };
        assert_eq!(resolve_conflict(&e, right_only), Some(SyncOp::CopyRightToLeft));

        // Neither trusted → nothing (the UI prevents this state).
        let neither = SyncFlags { trust_left: false, trust_right: false, ..both(ConflictRule::Newer) };
        assert_eq!(resolve_conflict(&e, neither), None);
    }

    #[test]
    fn fingerprint_is_stable_and_distinguishes() {
        assert_eq!(fingerprint(b"hello"), fingerprint(b"hello"));
        assert_ne!(fingerprint(b"hello"), fingerprint(b"world"));
    }

    #[test]
    fn detect_eol_picks_crlf_else_lf() {
        assert_eq!(detect_eol("a\r\nb"), "\r\n");
        assert_eq!(detect_eol("a\nb"), "\n");
        assert_eq!(detect_eol("no newline"), "\n");
    }

    #[test]
    fn read_capped_rejects_files_over_cap() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("small.txt"), b"ok").unwrap();
        assert!(read_capped(&fref(dir.path(), "small.txt"), TEXT_CAP).is_ok());
        fs::write(
            dir.path().join("big.bin"),
            vec![0u8; (TEXT_CAP + 1) as usize],
        )
        .unwrap();
        assert!(read_capped(&fref(dir.path(), "big.bin"), TEXT_CAP).is_err());
    }

    #[test]
    fn read_side_cached_treats_missing_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("here.txt"), b"hi").unwrap();
        let src = LocalSource::new(dir.path());
        assert_eq!(
            read_side_cached(&src, &rel_from_str("here.txt"), TEXT_CAP).unwrap().0,
            b"hi"
        );
        // A missing file (present on only one side) reads as empty, not an error.
        assert_eq!(
            read_side_cached(&src, &rel_from_str("gone.txt"), TEXT_CAP).unwrap().0,
            Vec::<u8>::new()
        );
    }

    #[test]
    fn rel_from_str_splits_on_slash() {
        assert_eq!(rel_from_str("a.txt"), RelPath::root().child("a.txt"));
        assert_eq!(
            rel_from_str("sub/dir/f.txt"),
            RelPath::root().child("sub").child("dir").child("f.txt")
        );
        assert!(rel_from_str("").is_root());
    }

    #[test]
    fn diff_file_distinguishes_text_and_binary() {
        let dir = tempfile::tempdir().unwrap();
        let src = LocalSource::new(dir.path());
        fs::write(dir.path().join("a.txt"), "x\ny\n").unwrap();
        fs::write(dir.path().join("b.txt"), "x\nz\n").unwrap();
        let res = diff_file_inner(&src, &rel_from_str("a.txt"), &src, &rel_from_str("b.txt")).unwrap();
        assert!(matches!(res.result, DiffResult::Text { .. }));
        assert!(!res.left.fp.is_empty() && !res.right.fp.is_empty());

        fs::write(dir.path().join("a.bin"), [0xff, 0xfe, 0x00]).unwrap();
        fs::write(dir.path().join("b.bin"), [0xff, 0xfe, 0x00]).unwrap();
        match diff_file_inner(&src, &rel_from_str("a.bin"), &src, &rel_from_str("b.bin"))
            .unwrap()
            .result
        {
            DiffResult::Binary { identical } => assert!(identical),
            _ => panic!("expected a binary verdict"),
        }
    }

    #[test]
    fn diff_file_signals_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let src = LocalSource::new(dir.path());
        fs::write(dir.path().join("big.txt"), vec![b'a'; (TEXT_CAP + 1) as usize]).unwrap();
        fs::write(dir.path().join("small.txt"), b"hi").unwrap();
        match diff_file_inner(&src, &rel_from_str("big.txt"), &src, &rel_from_str("small.txt")).unwrap().result {
            DiffResult::TooLarge { left_size, right_size } => {
                assert_eq!(left_size, TEXT_CAP + 1);
                assert_eq!(right_size, 2);
            }
            _ => panic!("expected TooLarge"),
        }
    }

    #[test]
    fn save_file_guards_conflicts_and_forces() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("c.txt"), "old").unwrap();
        let old_fp = fingerprint(b"old");
        let f = fref(dir.path(), "c.txt");
        let on_disk = || fs::read_to_string(dir.path().join("c.txt")).unwrap();

        // expected fp matches what's on disk → writes.
        assert!(matches!(
            save_file(f.clone(), "new".into(), Some(old_fp.clone()), false).unwrap(),
            SaveResult::Saved { .. }
        ));
        assert_eq!(on_disk(), "new");

        // file changed under us (stale expected fp) → conflict, nothing written.
        assert!(matches!(
            save_file(f.clone(), "newer".into(), Some(old_fp.clone()), false).unwrap(),
            SaveResult::Conflict
        ));
        assert_eq!(on_disk(), "new");

        // force overrides the conflict.
        assert!(matches!(
            save_file(f, "forced".into(), Some(old_fp), true).unwrap(),
            SaveResult::Saved { .. }
        ));
        assert_eq!(on_disk(), "forced");
    }

    #[test]
    fn save_file_creates_a_new_file_when_no_expected_fp() {
        let dir = tempfile::tempdir().unwrap();
        // No file on disk + expect=None → current(None)==expect(None) → writes a fresh file.
        assert!(matches!(
            save_file(fref(dir.path(), "new.txt"), "hi".into(), None, false).unwrap(),
            SaveResult::Saved { .. }
        ));
        assert_eq!(
            fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "hi"
        );
    }

    #[test]
    fn save_file_creates_missing_target_even_with_an_expected_fp() {
        // One-sided file: the UI read the (absent) other side as empty, so `expect` is the empty-bytes fp,
        // but the target doesn't exist → this is a create, NOT a conflict.
        let dir = tempfile::tempdir().unwrap();
        let empty_fp = fingerprint(b"");
        assert!(matches!(
            save_file(
                fref(dir.path(), "created.txt"),
                "made".into(),
                Some(empty_fp),
                false
            )
            .unwrap(),
            SaveResult::Saved { .. }
        ));
        assert_eq!(
            fs::read_to_string(dir.path().join("created.txt")).unwrap(),
            "made"
        );
    }

    #[test]
    fn hex_compare_reports_lengths_uncapped() {
        let dir = tempfile::tempdir().unwrap();
        let src = LocalSource::new(dir.path());
        fs::write(dir.path().join("l"), vec![1u8; 10]).unwrap();
        fs::write(dir.path().join("r"), vec![2u8; 20]).unwrap();
        let h = hex_compare_inner(&src, &rel_from_str("l"), &src, &rel_from_str("r"));
        assert_eq!((h.left_len, h.right_len), (10, 20));
        assert_eq!(h.left.len(), 10);
        assert!(!h.truncated);
    }

    #[test]
    fn hex_compare_truncates_large_file_without_rejecting() {
        let dir = tempfile::tempdir().unwrap();
        let src = LocalSource::new(dir.path());
        // A file bigger than HEX_CAP must NOT be rejected — only the prefix is read, truncated=true.
        fs::write(dir.path().join("big"), vec![7u8; HEX_CAP + 1000]).unwrap();
        fs::write(dir.path().join("small"), vec![7u8; 5]).unwrap();
        let h = hex_compare_inner(&src, &rel_from_str("big"), &src, &rel_from_str("small"));
        assert_eq!(h.left_len, (HEX_CAP + 1000) as u64);
        assert_eq!(h.left.len(), HEX_CAP); // prefix only
        assert!(h.truncated);
    }

    #[test]
    fn read_bytes_errors_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_bytes(fref(dir.path(), "nope")).is_err());
    }

    #[test]
    fn test_source_reports_dir_file_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f.txt"), b"hi").unwrap();

        let d = test_source(fs_spec(dir.path().to_str().unwrap().to_string()));
        assert!(d.ok && d.is_dir);

        let f = test_source(fs_spec(
            dir.path().join("f.txt").to_str().unwrap().to_string(),
        ));
        assert!(f.ok && !f.is_dir);

        let missing = test_source(fs_spec(
            dir.path().join("nope").to_str().unwrap().to_string(),
        ));
        assert!(!missing.ok);
    }

    #[test]
    fn source_types_catalog_lists_registered_kinds() {
        let types = source_types();
        let ids: Vec<_> = types.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["fs", "sftp", "s3"]);
        let sftp = types.iter().find(|t| t.id == "sftp").unwrap();
        assert!(sftp.capabilities.write);
        assert!(sftp.fields.iter().any(|f| f.key == "host"));
        // Credential fields are marked secret so the UI masks them.
        assert!(sftp.fields.iter().any(|f| f.secret));
        // S3 registered with a secret credential field and an icon — the picker renders it generically.
        let s3 = types.iter().find(|t| t.id == "s3").unwrap();
        assert!(!s3.icon.is_empty());
        assert!(s3.fields.iter().any(|f| f.key == "bucket"));
        assert!(s3.fields.iter().any(|f| f.key == "secret_access_key" && f.secret));
    }

    #[test]
    fn sftp_spec_deserializes_generic_and_maps_auth() {
        let json = serde_json::json!({
            "kind": "sftp",
            "fields": {
                "host": "example.com",
                "username": "alice",
                "auth.method": "password",
                "auth.password": "s3cret"
            }
        });
        let spec: SourceSpec = serde_json::from_value(json).unwrap();
        assert_eq!(spec.kind, "sftp");
        assert_eq!(spec.fields.get("host").map(String::as_str), Some("example.com"));
        // Auth maps to the domain type (defaults like port/root are applied at build time, not here).
        assert!(matches!(
            sftp_auth_from_fields(&spec.fields).unwrap(),
            SftpAuth::Password(p) if p == "s3cret"
        ));
        // The SFTP kind's cache key is secret-free and identity-stable.
        let key = kind_for("sftp").unwrap().cache_key(&spec.fields);
        assert!(key.starts_with("sftp:alice@example.com"));
        assert!(!key.contains("s3cret"));
    }
}
