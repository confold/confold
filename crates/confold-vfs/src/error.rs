//! Error type for VFS backends.

/// Failure reading from a [`Source`](crate::Source).
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// An underlying I/O error, annotated with the relative path it occurred on.
    #[error("I/O error at {path:?}: {source}")]
    Io {
        /// Relative path (as `a/b/c`) where the error happened.
        path: String,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
    /// The backend does not support this operation (per its [`Capabilities`](crate::Capabilities)) —
    /// e.g. listing or reading a write-only sink. The `&str` names the attempted op.
    #[error("operation not supported by this source: {0}")]
    Unsupported(&'static str),
    /// A backend-specific error that does not map to an I/O error.
    #[error("{0}")]
    Other(String),
}

impl SourceError {
    /// Build an [`SourceError::Io`] from a path-like and an [`std::io::Error`].
    pub fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        SourceError::Io {
            path: path.into(),
            source,
        }
    }
}
