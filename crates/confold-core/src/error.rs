//! Engine error type.

/// A fatal error from the compare engine.
///
/// Only failures at the **root** of the comparison surface as `Err` (e.g. a root path that cannot be
/// listed). Failures deeper in the tree are recorded as [`DiffStatus::Error`](crate::DiffStatus) nodes so
/// one unreadable file or directory never aborts the whole compare.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A VFS-level failure reading one of the roots.
    #[error(transparent)]
    Source(#[from] confold_vfs::SourceError),
}
