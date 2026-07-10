#[allow(unused_imports)]
use crate::scan::*;
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

/// A data source chosen in the UI: a registered kind id + flat config values. The generic wire DTO —
/// construction goes through the `SourceKind` registry below. Adding a backend never touches this type.
#[derive(Deserialize, Clone)]
pub(crate) struct SourceSpec {
    /// Registered source-kind id (e.g. `"fs"`, `"sftp"`), matching `SourceKind::id`.
    pub(crate) kind: String,
    /// Flat config values (dotted keys for nested fields, e.g. `auth.password`). Secrets live here too
    /// — never log them.
    #[serde(default)]
    pub(crate) fields: FieldValues,
}

pub(crate) fn default_sftp_port() -> u16 {
    22
}
pub(crate) fn default_sftp_root() -> String {
    "/".to_owned()
}

/// Connect an `SftpSource` from already-parsed SFTP config (shared by the read + write factories).
pub(crate) fn connect_sftp(
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
pub(crate) type FieldValues = BTreeMap<String, String>;

/// A registered source type: its identity, config form, capabilities, and how to construct it.
pub(crate) trait SourceKind: Send + Sync {
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
pub(crate) fn required_field(f: &FieldValues, key: &str) -> Result<String, String> {
    f.get(key)
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or_else(|| format!("missing required field '{key}'"))
}

/// An optional config value (treats empty as absent).
pub(crate) fn optional_field(f: &FieldValues, key: &str) -> Option<String> {
    f.get(key).filter(|s| !s.is_empty()).cloned()
}

/// Local filesystem source.
pub(crate) struct FsKind;
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
pub(crate) struct SftpKind;
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
pub(crate) struct S3Kind;
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
pub(crate) fn sftp_auth_from_fields(f: &FieldValues) -> Result<SftpAuth, String> {
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
pub(crate) static REGISTRY: LazyLock<Vec<Box<dyn SourceKind>>> =
    LazyLock::new(|| vec![Box::new(FsKind), Box::new(SftpKind), Box::new(S3Kind)]);

/// Look up a registered source kind by id.
pub(crate) fn kind_for(id: &str) -> Result<&'static dyn SourceKind, String> {
    REGISTRY
        .iter()
        .find(|k| k.id() == id)
        .map(|k| &**k)
        .ok_or_else(|| format!("unknown source type '{id}'"))
}

/// Build a read source from a spec (via the registry).
pub(crate) fn build_source(spec: &SourceSpec) -> Result<Box<dyn Source>, String> {
    kind_for(&spec.kind)?.build_source(&spec.fields)
}

/// Build a writable source from a spec — errors if the type can't be written to.
pub(crate) fn build_source_mut(spec: &SourceSpec) -> Result<Box<dyn SourceMut>, String> {
    kind_for(&spec.kind)?.build_source_mut(&spec.fields)
}
