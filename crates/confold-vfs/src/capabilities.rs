//! What a source can do — drives which operations the engine and UI offer for it.

use serde::{Deserialize, Serialize};

/// The operations a [`Source`](crate::Source) supports.
///
/// The UI gates actions on these instead of guessing from which traits are implemented, and the engine
/// can check them before calling. This lets odd shapes exist cleanly — e.g. a fingerprint-only source
/// (metadata triage, no byte reads) or a **write-only sink** (a secrets vault you can push into but not
/// read back), which is a migration *target* only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Can enumerate a directory tree (`read_dir`). Needed to compare *folders*.
    pub list: bool,
    /// Can read file contents (`open`). Needed to compare *content* / open a file viewer.
    pub read: bool,
    /// Exposes a cheap content fingerprint (`quick_fingerprint`) to skip byte reads.
    pub fingerprint: bool,
    /// Can create / overwrite / delete (`SourceMut`). Needed as a sync / migration destination.
    pub write: bool,
}

impl Capabilities {
    /// A read + write source with no cheap fingerprint (e.g. a local filesystem).
    pub const FS_RW: Capabilities = Capabilities {
        list: true,
        read: true,
        fingerprint: false,
        write: true,
    };

    /// Folder-tree compare needs listing **and** a way to tell files apart (read or fingerprint).
    pub fn allows_tree_compare(&self) -> bool {
        self.list && (self.read || self.fingerprint)
    }

    /// File-content compare / opening a viewer needs reading.
    pub fn allows_content_compare(&self) -> bool {
        self.read
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_rw_allows_everything() {
        let c = Capabilities::FS_RW;
        assert!(c.allows_tree_compare());
        assert!(c.allows_content_compare());
        assert!(c.write);
    }

    #[test]
    fn write_only_sink_blocks_compare_but_allows_write() {
        let c = Capabilities {
            list: false,
            read: false,
            fingerprint: false,
            write: true,
        };
        assert!(!c.allows_tree_compare());
        assert!(!c.allows_content_compare());
        assert!(c.write);
    }

    #[test]
    fn fingerprint_only_allows_tree_compare_not_content() {
        let c = Capabilities {
            list: true,
            read: false,
            fingerprint: true,
            write: false,
        };
        assert!(c.allows_tree_compare());
        assert!(!c.allows_content_compare());
    }
}
