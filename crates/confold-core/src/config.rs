//! Comparison configuration.

use crate::filter::FilterSet;

/// Files larger than this use sampled comparison under [`CompareMethod::Quick`]. Matches the common
/// default quick-contents threshold (4 MiB).
pub const DEFAULT_LARGE_FILE_THRESHOLD: u64 = 4 * 1024 * 1024;

/// How two files are judged equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareMethod {
    /// Equal iff byte sizes match. No content read.
    Size,
    /// Equal iff modified times match. No content read.
    Mtime,
    /// Equal iff both size and modified time match. No content read.
    SizeAndMtime,
    /// Byte-by-byte over the whole file (after a size pre-check).
    Full,
    /// Like [`Full`](CompareMethod::Full) up to `large_file_threshold`; above it, compare a bounded
    /// sample (head/tail + sampled blocks). Faster on large files, with sampling uncertainty surfaced in
    /// the result detail.
    Quick {
        /// Size (bytes) above which sampled comparison kicks in.
        large_file_threshold: u64,
    },
}

impl CompareMethod {
    /// `true` if the method never reads file contents (metadata-only).
    pub fn is_metadata_only(self) -> bool {
        matches!(
            self,
            CompareMethod::Size | CompareMethod::Mtime | CompareMethod::SizeAndMtime
        )
    }
}

/// Full configuration for a comparison run.
#[derive(Clone, Debug)]
pub struct CompareConfig {
    /// Content/metadata comparison method.
    pub method: CompareMethod,
    /// Descend into subdirectories. When `false`, subdirectories present on both sides are listed but
    /// reported as `Skipped` ("not descended").
    pub recursive: bool,
    /// Include/exclude globs.
    pub filters: FilterSet,
}

impl Default for CompareConfig {
    fn default() -> Self {
        CompareConfig {
            method: CompareMethod::Quick {
                large_file_threshold: DEFAULT_LARGE_FILE_THRESHOLD,
            },
            recursive: true,
            filters: FilterSet::default(),
        }
    }
}
