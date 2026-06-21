//! Confold compare engine.
//!
//! Compares two directory trees exposed as [`confold_vfs::Source`]s, classifying each item and (for files)
//! comparing contents with a configurable [`CompareMethod`]. Produces a structured [`DiffReport`].
//!
//! ```no_run
//! use confold_core::{compare, CompareConfig};
//! use confold_vfs::LocalSource;
//!
//! let left = LocalSource::new("/path/a");
//! let right = LocalSource::new("/path/b");
//! let report = compare(&left, &right, &CompareConfig::default()).unwrap();
//! println!("{}", confold_core::render_text(&report));
//! ```

mod compare;
mod config;
mod content;
mod error;
mod filter;
mod model;
mod render;

pub use compare::{compare, compare_at, compare_at_with_progress, compare_file, list_level, ProgressFn};
pub use content::full_equal;
pub use config::{CompareConfig, CompareMethod, DEFAULT_LARGE_FILE_THRESHOLD};
pub use error::EngineError;
pub use filter::FilterSet;
pub use model::{DiffEntry, DiffReport, DiffStatus, Summary};
pub use render::render_text;

// Re-export the VFS surface callers need so they can depend on just `confold-core`.
pub use confold_vfs::{
    Capabilities, ContentReader, EntryKind, EntryMeta, LocalSource, RelPath, Source, SourceError,
    SourceMut,
};
