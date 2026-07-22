//! Safe protocol boundary for AI-assisted semantic comparison and merge proposals.
//!
//! Confold captures bounded immutable input snapshots, an agent proposes a semantic result in JSON,
//! and this crate validates the proposal against fresh input fingerprints before producing a review
//! or writing a new output file. It performs no model invocation and never overwrites an input.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use tempfile::NamedTempFile;
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_INPUT_BYTES: u64 = 2_000_000;
pub const MAX_RESULT_BYTES: u64 = MAX_INPUT_BYTES * 3;
pub const MAX_PROTOCOL_JSON_BYTES: u64 = MAX_RESULT_BYTES * 6 + 1_000_000;
pub const SUPPORTED_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "rst", "adoc", "asciidoc"];

#[derive(Debug, Error)]
pub enum SemanticError {
    #[error("unsupported schema version {actual}; expected {expected}")]
    UnsupportedSchema { actual: u32, expected: u32 },
    #[error("input path does not exist or cannot be resolved: {path}: {source}")]
    Canonicalize {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("input is not a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("unsupported input extension for {path}; supported: {supported}")]
    UnsupportedExtension { path: PathBuf, supported: String },
    #[error("input exceeds the {limit}-byte semantic limit: {path} ({actual} bytes)")]
    TooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("input is not valid UTF-8 text: {0}")]
    InvalidUtf8(PathBuf),
    #[error("input appears to be binary text because it contains NUL bytes: {0}")]
    Binary(PathBuf),
    #[error("semantic inputs must resolve to distinct files: {0}")]
    DuplicateInput(PathBuf),
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse JSON from {path}: {source}")]
    ParseJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("protocol JSON exceeds the {limit}-byte limit: {path} ({actual} bytes)")]
    ProtocolJsonTooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("failed to serialize JSON: {0}")]
    SerializeJson(serde_json::Error),
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("operation id mismatch: bundle has {bundle}, proposal has {proposal}")]
    OperationMismatch { bundle: String, proposal: String },
    #[error("proposal is invalid: {0}")]
    InvalidProposal(String),
    #[error("input changed after prepare: {path}")]
    StaleInput { path: PathBuf },
    #[error("proposal verdict {0:?} does not produce an applicable result")]
    NotApplicable(Verdict),
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRole {
    Left,
    Right,
    Base,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EolStyle {
    None,
    Lf,
    Crlf,
    Cr,
    Mixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSnapshot {
    pub role: InputRole,
    pub path: PathBuf,
    pub sha256: String,
    pub byte_len: u64,
    pub eol: EolStyle,
    pub final_newline: bool,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FastPath {
    NeedsSemanticAnalysis,
    ByteIdentical,
    FormattingOnly,
    PreferLeft,
    PreferRight,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticBundle {
    pub schema_version: u32,
    pub operation_id: String,
    pub inputs: Vec<InputSnapshot>,
    pub fast_path: FastPath,
    pub proposal_schema_version: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Equivalent,
    PreferLeft,
    PreferRight,
    Merged,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Preserved,
    AlreadyPresent,
    Superseded,
    Omitted,
    Uncertain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub source: InputRole,
    pub intent: String,
    pub disposition: Disposition,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticProposal {
    pub schema_version: u32,
    pub operation_id: String,
    pub verdict: Verdict,
    pub summary: String,
    pub contributions: Vec<Contribution>,
    pub warnings: Vec<String>,
    pub result: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReviewDiff {
    pub source: InputRole,
    pub unified_diff: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReviewReport {
    pub schema_version: u32,
    pub operation_id: String,
    pub verdict: Verdict,
    pub applicable: bool,
    pub summary: String,
    pub contributions: Vec<Contribution>,
    pub warnings: Vec<String>,
    pub result_sha256: Option<String>,
    pub diffs: Vec<ReviewDiff>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ApplyReport {
    pub schema_version: u32,
    pub operation_id: String,
    pub output: PathBuf,
    pub sha256: String,
    pub byte_len: u64,
}

pub fn prepare(
    left: &Path,
    right: &Path,
    base: Option<&Path>,
) -> Result<SemanticBundle, SemanticError> {
    let mut inputs = vec![
        snapshot(InputRole::Left, left)?,
        snapshot(InputRole::Right, right)?,
    ];
    if let Some(path) = base {
        inputs.push(snapshot(InputRole::Base, path)?);
    }
    ensure_distinct(&inputs)?;
    let fast_path = classify_fast_path(&inputs);
    Ok(SemanticBundle {
        schema_version: SCHEMA_VERSION,
        operation_id: operation_id(&inputs),
        inputs,
        fast_path,
        proposal_schema_version: SCHEMA_VERSION,
    })
}

pub fn review(
    bundle: &SemanticBundle,
    proposal: &SemanticProposal,
) -> Result<ReviewReport, SemanticError> {
    validate(bundle, proposal)?;
    ensure_fresh(bundle)?;
    let result = applicable_result(bundle, proposal)?;
    let result_sha256 = result.map(|text| sha256(text.as_bytes()));
    let diffs = result
        .map(|text| {
            bundle
                .inputs
                .iter()
                .map(|input| ReviewDiff {
                    source: input.role,
                    unified_diff: unified_diff(input.role, &input.content, text),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ReviewReport {
        schema_version: SCHEMA_VERSION,
        operation_id: bundle.operation_id.clone(),
        verdict: proposal.verdict,
        applicable: result.is_some(),
        summary: proposal.summary.clone(),
        contributions: proposal.contributions.clone(),
        warnings: proposal.warnings.clone(),
        result_sha256,
        diffs,
    })
}

pub fn apply(
    bundle: &SemanticBundle,
    proposal: &SemanticProposal,
    output: &Path,
) -> Result<ApplyReport, SemanticError> {
    validate(bundle, proposal)?;
    ensure_fresh(bundle)?;
    let result = applicable_result(bundle, proposal)?
        .ok_or(SemanticError::NotApplicable(proposal.verdict))?;
    let output = absolute_output(output)?;
    ensure_not_an_input(bundle, &output)?;
    write_new_atomic(&output, result.as_bytes())?;
    Ok(ApplyReport {
        schema_version: SCHEMA_VERSION,
        operation_id: bundle.operation_id.clone(),
        output,
        sha256: sha256(result.as_bytes()),
        byte_len: result.len() as u64,
    })
}

pub fn read_bundle(path: &Path) -> Result<SemanticBundle, SemanticError> {
    read_json(path)
}

pub fn read_proposal(path: &Path) -> Result<SemanticProposal, SemanticError> {
    read_json(path)
}

pub fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<(), SemanticError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(SemanticError::SerializeJson)?;
    write_new_atomic(path, &bytes)
}

fn snapshot(role: InputRole, path: &Path) -> Result<InputSnapshot, SemanticError> {
    let canonical = fs::canonicalize(path).map_err(|source| SemanticError::Canonicalize {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = fs::metadata(&canonical).map_err(|source| SemanticError::Read {
        path: canonical.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(SemanticError::NotAFile(canonical));
    }
    validate_extension(&canonical)?;
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(SemanticError::TooLarge {
            path: canonical,
            actual: metadata.len(),
            limit: MAX_INPUT_BYTES,
        });
    }
    let bytes = fs::read(&canonical).map_err(|source| SemanticError::Read {
        path: canonical.clone(),
        source,
    })?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(SemanticError::TooLarge {
            path: canonical,
            actual: bytes.len() as u64,
            limit: MAX_INPUT_BYTES,
        });
    }
    if bytes.contains(&0) {
        return Err(SemanticError::Binary(canonical));
    }
    let content = String::from_utf8(bytes.clone())
        .map_err(|_| SemanticError::InvalidUtf8(canonical.clone()))?;
    Ok(InputSnapshot {
        role,
        path: canonical,
        sha256: sha256(&bytes),
        byte_len: bytes.len() as u64,
        eol: detect_eol(&bytes),
        final_newline: bytes.ends_with(b"\n") || bytes.ends_with(b"\r"),
        content,
    })
}

fn validate_extension(path: &Path) -> Result<(), SemanticError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if extension
        .as_deref()
        .is_some_and(|value| SUPPORTED_EXTENSIONS.contains(&value))
    {
        Ok(())
    } else {
        Err(SemanticError::UnsupportedExtension {
            path: path.to_path_buf(),
            supported: SUPPORTED_EXTENSIONS.join(", "),
        })
    }
}

fn ensure_distinct(inputs: &[InputSnapshot]) -> Result<(), SemanticError> {
    let mut paths = HashSet::new();
    for input in inputs {
        if !paths.insert(&input.path) {
            return Err(SemanticError::DuplicateInput(input.path.clone()));
        }
    }
    Ok(())
}

fn classify_fast_path(inputs: &[InputSnapshot]) -> FastPath {
    let left = input(inputs, InputRole::Left).expect("left snapshot is mandatory");
    let right = input(inputs, InputRole::Right).expect("right snapshot is mandatory");
    if left.sha256 == right.sha256 {
        return FastPath::ByteIdentical;
    }
    if normalize_text(&left.content) == normalize_text(&right.content) {
        return FastPath::FormattingOnly;
    }
    if let Some(base) = input(inputs, InputRole::Base) {
        if left.sha256 == base.sha256 && right.sha256 != base.sha256 {
            return FastPath::PreferRight;
        }
        if right.sha256 == base.sha256 && left.sha256 != base.sha256 {
            return FastPath::PreferLeft;
        }
    }
    FastPath::NeedsSemanticAnalysis
}

fn validate(bundle: &SemanticBundle, proposal: &SemanticProposal) -> Result<(), SemanticError> {
    validate_bundle(bundle)?;
    check_schema(proposal.schema_version)?;
    if bundle.operation_id != proposal.operation_id {
        return Err(SemanticError::OperationMismatch {
            bundle: bundle.operation_id.clone(),
            proposal: proposal.operation_id.clone(),
        });
    }
    if proposal.summary.trim().is_empty() {
        return Err(SemanticError::InvalidProposal(
            "summary must not be empty".into(),
        ));
    }
    match proposal.verdict {
        Verdict::Merged if proposal.result.is_none() => {
            return Err(SemanticError::InvalidProposal(
                "merged verdict requires result text".into(),
            ));
        }
        Verdict::Equivalent | Verdict::PreferLeft | Verdict::PreferRight | Verdict::Uncertain
            if proposal.result.is_some() =>
        {
            return Err(SemanticError::InvalidProposal(format!(
                "{:?} verdict must not include result text",
                proposal.verdict
            )));
        }
        _ => {}
    }
    let expected_fast_path_verdict = match bundle.fast_path {
        FastPath::ByteIdentical | FastPath::FormattingOnly => Some(Verdict::Equivalent),
        FastPath::PreferLeft => Some(Verdict::PreferLeft),
        FastPath::PreferRight => Some(Verdict::PreferRight),
        FastPath::NeedsSemanticAnalysis => None,
    };
    if let Some(expected) = expected_fast_path_verdict {
        if proposal.verdict != expected && proposal.verdict != Verdict::Uncertain {
            return Err(SemanticError::InvalidProposal(format!(
                "fast path {:?} requires {:?} or Uncertain, got {:?}",
                bundle.fast_path, expected, proposal.verdict
            )));
        }
    }
    if proposal
        .result
        .as_ref()
        .is_some_and(|result| result.len() as u64 > MAX_RESULT_BYTES)
    {
        return Err(SemanticError::InvalidProposal(format!(
            "merged result exceeds the {MAX_RESULT_BYTES}-byte semantic limit"
        )));
    }
    let roles: HashSet<InputRole> = bundle.inputs.iter().map(|input| input.role).collect();
    for contribution in &proposal.contributions {
        if !roles.contains(&contribution.source) {
            return Err(SemanticError::InvalidProposal(format!(
                "contribution references absent source {:?}",
                contribution.source
            )));
        }
        if contribution.intent.trim().is_empty() {
            return Err(SemanticError::InvalidProposal(
                "contribution intent must not be empty".into(),
            ));
        }
    }
    if bundle.fast_path == FastPath::NeedsSemanticAnalysis {
        for required in [InputRole::Left, InputRole::Right] {
            if !proposal
                .contributions
                .iter()
                .any(|contribution| contribution.source == required)
            {
                return Err(SemanticError::InvalidProposal(format!(
                    "semantic analysis requires a contribution for {required:?}"
                )));
            }
        }
    }
    Ok(())
}

fn validate_bundle(bundle: &SemanticBundle) -> Result<(), SemanticError> {
    check_schema(bundle.schema_version)?;
    check_schema(bundle.proposal_schema_version)?;
    if !(bundle.inputs.len() == 2 || bundle.inputs.len() == 3) {
        return Err(SemanticError::InvalidProposal(
            "bundle must contain left and right inputs plus at most one base".into(),
        ));
    }
    ensure_distinct(&bundle.inputs)?;
    for role in [InputRole::Left, InputRole::Right] {
        if bundle
            .inputs
            .iter()
            .filter(|input| input.role == role)
            .count()
            != 1
        {
            return Err(SemanticError::InvalidProposal(format!(
                "bundle must contain exactly one {role:?} input"
            )));
        }
    }
    if bundle
        .inputs
        .iter()
        .filter(|input| input.role == InputRole::Base)
        .count()
        > 1
    {
        return Err(SemanticError::InvalidProposal(
            "bundle must contain at most one base input".into(),
        ));
    }
    for input in &bundle.inputs {
        if input.sha256 != sha256(input.content.as_bytes())
            || input.byte_len != input.content.len() as u64
            || input.eol != detect_eol(input.content.as_bytes())
            || input.final_newline
                != (input.content.as_bytes().ends_with(b"\n")
                    || input.content.as_bytes().ends_with(b"\r"))
        {
            return Err(SemanticError::InvalidProposal(format!(
                "bundle snapshot metadata does not match {:?} content",
                input.role
            )));
        }
    }
    if bundle.fast_path != classify_fast_path(&bundle.inputs) {
        return Err(SemanticError::InvalidProposal(
            "bundle fast_path does not match its snapshots".into(),
        ));
    }
    if bundle.operation_id != operation_id(&bundle.inputs) {
        return Err(SemanticError::InvalidProposal(
            "bundle operation_id does not match its snapshots".into(),
        ));
    }
    Ok(())
}

fn check_schema(actual: u32) -> Result<(), SemanticError> {
    if actual == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(SemanticError::UnsupportedSchema {
            actual,
            expected: SCHEMA_VERSION,
        })
    }
}

fn ensure_fresh(bundle: &SemanticBundle) -> Result<(), SemanticError> {
    for snapshot in &bundle.inputs {
        let bytes = match fs::read(&snapshot.path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(SemanticError::StaleInput {
                    path: snapshot.path.clone(),
                });
            }
            Err(source) => {
                return Err(SemanticError::Read {
                    path: snapshot.path.clone(),
                    source,
                });
            }
        };
        if sha256(&bytes) != snapshot.sha256 {
            return Err(SemanticError::StaleInput {
                path: snapshot.path.clone(),
            });
        }
    }
    Ok(())
}

fn applicable_result<'a>(
    bundle: &'a SemanticBundle,
    proposal: &'a SemanticProposal,
) -> Result<Option<&'a str>, SemanticError> {
    let result = match proposal.verdict {
        Verdict::PreferLeft => {
            input(&bundle.inputs, InputRole::Left).map(|value| value.content.as_str())
        }
        Verdict::PreferRight => {
            input(&bundle.inputs, InputRole::Right).map(|value| value.content.as_str())
        }
        Verdict::Merged => proposal.result.as_deref(),
        Verdict::Equivalent | Verdict::Uncertain => None,
    };
    Ok(result)
}

fn input(inputs: &[InputSnapshot], role: InputRole) -> Option<&InputSnapshot> {
    inputs.iter().find(|input| input.role == role)
}

fn absolute_output(path: &Path) -> Result<PathBuf, SemanticError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| SemanticError::Canonicalize {
                path: path.to_path_buf(),
                source,
            })?
            .join(path)
    };
    let parent = absolute.parent().unwrap_or_else(|| Path::new("."));
    let canonical_parent =
        fs::canonicalize(parent).map_err(|source| SemanticError::Canonicalize {
            path: parent.to_path_buf(),
            source,
        })?;
    let name = absolute.file_name().ok_or_else(|| {
        SemanticError::InvalidProposal(format!("output has no file name: {}", absolute.display()))
    })?;
    Ok(canonical_parent.join(name))
}

fn ensure_not_an_input(bundle: &SemanticBundle, output: &Path) -> Result<(), SemanticError> {
    if bundle.inputs.iter().any(|input| input.path == output) {
        return Err(SemanticError::InvalidProposal(format!(
            "output must not replace an input: {}",
            output.display()
        )));
    }
    Ok(())
}

fn write_new_atomic(path: &Path, bytes: &[u8]) -> Result<(), SemanticError> {
    if path.exists() {
        return Err(SemanticError::OutputExists(path.to_path_buf()));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| SemanticError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    temporary
        .write_all(bytes)
        .map_err(|source| SemanticError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| SemanticError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    temporary.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            SemanticError::OutputExists(path.to_path_buf())
        } else {
            SemanticError::Write {
                path: path.to_path_buf(),
                source: error.error,
            }
        }
    })?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, SemanticError> {
    let metadata = fs::metadata(path).map_err(|source| SemanticError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_PROTOCOL_JSON_BYTES {
        return Err(SemanticError::ProtocolJsonTooLarge {
            path: path.to_path_buf(),
            actual: metadata.len(),
            limit: MAX_PROTOCOL_JSON_BYTES,
        });
    }
    let bytes = fs::read(path).map_err(|source| SemanticError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() as u64 > MAX_PROTOCOL_JSON_BYTES {
        return Err(SemanticError::ProtocolJsonTooLarge {
            path: path.to_path_buf(),
            actual: bytes.len() as u64,
            limit: MAX_PROTOCOL_JSON_BYTES,
        });
    }
    serde_json::from_slice(&bytes).map_err(|source| SemanticError::ParseJson {
        path: path.to_path_buf(),
        source,
    })
}

fn operation_id(inputs: &[InputSnapshot]) -> String {
    let mut digest = Sha256::new();
    for input in inputs {
        digest.update(match input.role {
            InputRole::Left => b"left".as_slice(),
            InputRole::Right => b"right".as_slice(),
            InputRole::Base => b"base".as_slice(),
        });
        digest.update([0]);
        digest.update(input.path.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(input.sha256.as_bytes());
        digest.update([0]);
    }
    hex(&digest.finalize())[..32].to_string()
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn detect_eol(bytes: &[u8]) -> EolStyle {
    let mut lf = 0usize;
    let mut crlf = 0usize;
    let mut cr = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf += 1;
                index += 2;
            }
            b'\r' => {
                cr += 1;
                index += 1;
            }
            b'\n' => {
                lf += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    match (lf > 0, crlf > 0, cr > 0) {
        (false, false, false) => EolStyle::None,
        (true, false, false) => EolStyle::Lf,
        (false, true, false) => EolStyle::Crlf,
        (false, false, true) => EolStyle::Cr,
        _ => EolStyle::Mixed,
    }
}

fn unified_diff(role: InputRole, original: &str, result: &str) -> String {
    TextDiff::from_lines(original, result)
        .unified_diff()
        .header(&format!("{role:?}"), "result")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &[u8]) {
        fs::write(path, content).unwrap();
    }

    fn proposal(
        bundle: &SemanticBundle,
        verdict: Verdict,
        result: Option<&str>,
    ) -> SemanticProposal {
        SemanticProposal {
            schema_version: SCHEMA_VERSION,
            operation_id: bundle.operation_id.clone(),
            verdict,
            summary: "Reviewed both documents".into(),
            contributions: vec![
                Contribution {
                    source: InputRole::Left,
                    intent: "Account for the left document".into(),
                    disposition: Disposition::Preserved,
                },
                Contribution {
                    source: InputRole::Right,
                    intent: "Account for the right document".into(),
                    disposition: Disposition::Preserved,
                },
            ],
            warnings: vec![],
            result: result.map(str::to_string),
        }
    }

    #[test]
    fn fast_paths_cover_identical_formatting_and_one_sided_base_change() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        let base = dir.path().join("base.md");
        write(&left, b"same\n");
        write(&right, b"same\n");
        assert_eq!(
            prepare(&left, &right, None).unwrap().fast_path,
            FastPath::ByteIdentical
        );

        write(&right, b"same  \r\n");
        assert_eq!(
            prepare(&left, &right, None).unwrap().fast_path,
            FastPath::FormattingOnly
        );

        write(&base, b"same\n");
        write(&right, b"changed\n");
        assert_eq!(
            prepare(&left, &right, Some(&base)).unwrap().fast_path,
            FastPath::PreferRight
        );
    }

    #[test]
    fn prepare_rejects_duplicate_binary_and_unsupported_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("doc.md");
        let binary = dir.path().join("binary.md");
        let json = dir.path().join("data.json");
        write(&text, b"text");
        write(&binary, b"a\0b");
        write(&json, b"{}");
        assert!(matches!(
            prepare(&text, &text, None),
            Err(SemanticError::DuplicateInput(_))
        ));
        assert!(matches!(
            prepare(&text, &binary, None),
            Err(SemanticError::Binary(_))
        ));
        assert!(matches!(
            prepare(&text, &json, None),
            Err(SemanticError::UnsupportedExtension { .. })
        ));
    }

    #[test]
    fn prepare_rejects_oversized_and_invalid_utf8_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let valid = dir.path().join("valid.md");
        let oversized = dir.path().join("oversized.md");
        let invalid_utf8 = dir.path().join("invalid-utf8.md");
        write(&valid, b"valid");
        write(&oversized, &vec![b'x'; MAX_INPUT_BYTES as usize + 1]);
        write(&invalid_utf8, &[0xff, 0xfe]);

        assert!(matches!(
            prepare(&valid, &oversized, None),
            Err(SemanticError::TooLarge {
                actual,
                limit: MAX_INPUT_BYTES,
                ..
            }) if actual == MAX_INPUT_BYTES + 1
        ));
        assert!(matches!(
            prepare(&valid, &invalid_utf8, None),
            Err(SemanticError::InvalidUtf8(_))
        ));
    }

    #[test]
    fn snapshots_preserve_eol_style_and_final_newline_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("none.md", b"one line".as_slice(), EolStyle::None, false),
            ("lf.md", b"one\ntwo\n".as_slice(), EolStyle::Lf, true),
            (
                "crlf.md",
                b"one\r\ntwo\r\n".as_slice(),
                EolStyle::Crlf,
                true,
            ),
            ("cr.md", b"one\rtwo\r".as_slice(), EolStyle::Cr, true),
            (
                "mixed.md",
                b"one\r\ntwo\nthree".as_slice(),
                EolStyle::Mixed,
                false,
            ),
        ];

        for (name, content, expected_eol, expected_final_newline) in cases {
            let path = dir.path().join(name);
            write(&path, content);
            let input = snapshot(InputRole::Left, &path).unwrap();
            assert_eq!(input.eol, expected_eol, "unexpected EOL style for {name}");
            assert_eq!(
                input.final_newline, expected_final_newline,
                "unexpected final-newline metadata for {name}"
            );
        }
    }

    #[test]
    fn proposal_json_rejects_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        let proposal_path = dir.path().join("proposal.json");
        write(&left, b"left");
        write(&right, b"right");
        let bundle = prepare(&left, &right, None).unwrap();
        let proposal = proposal(&bundle, Verdict::Uncertain, None);
        let mut json = serde_json::to_value(proposal).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert("contributons".into(), serde_json::json!([]));
        write(
            &proposal_path,
            serde_json::to_string_pretty(&json).unwrap().as_bytes(),
        );

        assert!(matches!(
            read_proposal(&proposal_path),
            Err(SemanticError::ParseJson { .. })
        ));
    }

    #[test]
    fn protocol_json_reads_are_bounded_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let proposal_path = dir.path().join("oversized-proposal.json");
        let file = fs::File::create(&proposal_path).unwrap();
        file.set_len(MAX_PROTOCOL_JSON_BYTES + 1).unwrap();

        let error = read_proposal(&proposal_path).unwrap_err();
        assert!(
            error.to_string().contains("protocol JSON exceeds"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn fast_path_rejects_a_contradictory_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        let base = dir.path().join("base.md");
        write(&left, b"original\n");
        write(&base, b"original\n");
        write(&right, b"updated\n");
        let bundle = prepare(&left, &right, Some(&base)).unwrap();
        assert_eq!(bundle.fast_path, FastPath::PreferRight);
        let proposal = proposal(&bundle, Verdict::PreferLeft, None);

        assert!(matches!(
            review(&bundle, &proposal),
            Err(SemanticError::InvalidProposal(_))
        ));
    }

    #[test]
    fn semantic_analysis_requires_left_and_right_contributions() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        write(&left, b"left intent\n");
        write(&right, b"right intent\n");
        let bundle = prepare(&left, &right, None).unwrap();
        assert_eq!(bundle.fast_path, FastPath::NeedsSemanticAnalysis);
        let mut proposal = proposal(&bundle, Verdict::Merged, Some("both intents\n"));
        proposal.contributions.clear();

        assert!(matches!(
            review(&bundle, &proposal),
            Err(SemanticError::InvalidProposal(_))
        ));
    }

    #[test]
    fn merged_result_rejects_output_larger_than_all_bounded_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        write(&left, b"left intent\n");
        write(&right, b"right intent\n");
        let bundle = prepare(&left, &right, None).unwrap();
        let oversized = "x".repeat(MAX_INPUT_BYTES as usize * 3 + 1);
        let mut proposal = proposal(&bundle, Verdict::Merged, Some(&oversized));
        proposal.contributions = vec![
            Contribution {
                source: InputRole::Left,
                intent: "Preserve the left intent".into(),
                disposition: Disposition::Preserved,
            },
            Contribution {
                source: InputRole::Right,
                intent: "Preserve the right intent".into(),
                disposition: Disposition::Preserved,
            },
        ];

        assert!(matches!(
            review(&bundle, &proposal),
            Err(SemanticError::InvalidProposal(_))
        ));
    }

    #[test]
    fn merged_proposal_reviews_and_writes_new_output() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        let output = dir.path().join("merged.md");
        write(&left, b"# Left\n");
        write(&right, b"# Right\n");
        let bundle = prepare(&left, &right, None).unwrap();
        let proposal = proposal(&bundle, Verdict::Merged, Some("# Left and right\n"));
        let review = review(&bundle, &proposal).unwrap();
        assert!(review.applicable);
        assert_eq!(review.diffs.len(), 2);
        let applied = apply(&bundle, &proposal, &output).unwrap();
        assert_eq!(applied.output, fs::canonicalize(&output).unwrap());
        assert_eq!(fs::read_to_string(&output).unwrap(), "# Left and right\n");
        assert!(matches!(
            apply(&bundle, &proposal, &output),
            Err(SemanticError::OutputExists(_))
        ));
    }

    #[test]
    fn stale_input_and_input_overwrite_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        write(&left, b"left\n");
        write(&right, b"right\n");
        let bundle = prepare(&left, &right, None).unwrap();
        let proposal = proposal(&bundle, Verdict::PreferLeft, None);
        assert!(matches!(
            apply(&bundle, &proposal, &left),
            Err(SemanticError::InvalidProposal(_))
        ));
        write(&right, b"changed after prepare\n");
        assert!(matches!(
            review(&bundle, &proposal),
            Err(SemanticError::StaleInput { .. })
        ));
    }

    #[test]
    fn invalid_proposal_combinations_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        write(&left, b"left");
        write(&right, b"right");
        let bundle = prepare(&left, &right, None).unwrap();
        let missing = proposal(&bundle, Verdict::Merged, None);
        assert!(matches!(
            review(&bundle, &missing),
            Err(SemanticError::InvalidProposal(_))
        ));
        let uncertain = proposal(&bundle, Verdict::Uncertain, Some("unsafe"));
        assert!(matches!(
            review(&bundle, &uncertain),
            Err(SemanticError::InvalidProposal(_))
        ));
    }

    #[test]
    fn tampered_bundle_and_deleted_input_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.md");
        let right = dir.path().join("right.md");
        write(&left, b"left");
        write(&right, b"right");
        let bundle = prepare(&left, &right, None).unwrap();
        let proposal = proposal(&bundle, Verdict::PreferLeft, None);

        let mut tampered = bundle.clone();
        input_mut(&mut tampered.inputs, InputRole::Left).content = "tampered".into();
        assert!(matches!(
            review(&tampered, &proposal),
            Err(SemanticError::InvalidProposal(_))
        ));

        let mut tampered_id = bundle.clone();
        tampered_id.operation_id = "00000000000000000000000000000000".into();
        let mut matching_proposal = proposal.clone();
        matching_proposal.operation_id = tampered_id.operation_id.clone();
        assert!(matches!(
            review(&tampered_id, &matching_proposal),
            Err(SemanticError::InvalidProposal(_))
        ));

        fs::remove_file(&right).unwrap();
        assert!(matches!(
            review(&bundle, &proposal),
            Err(SemanticError::StaleInput { .. })
        ));
    }

    fn input_mut(inputs: &mut [InputSnapshot], role: InputRole) -> &mut InputSnapshot {
        inputs.iter_mut().find(|input| input.role == role).unwrap()
    }
}
