//! Backend-neutral relative paths.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A path relative to a compared root, stored as ordered components.
///
/// This is the **match key** between two [`Source`](crate::Source)s: the same `RelPath` on the left and
/// right identifies a pair of items to compare. Components avoid separator ambiguity across platforms and
/// backends; [`Display`](std::fmt::Display) renders them joined by `/`. The root is the empty path.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RelPath(Vec<String>);

impl RelPath {
    /// The root (empty) relative path.
    pub fn root() -> Self {
        RelPath(Vec::new())
    }

    /// `true` if this is the root path.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Return a new `RelPath` with `name` appended as a child component.
    pub fn child(&self, name: &str) -> Self {
        let mut components = self.0.clone();
        components.push(name.to_owned());
        RelPath(components)
    }

    /// The ordered components.
    pub fn components(&self) -> &[String] {
        &self.0
    }

    /// The final component (file/dir name), or `None` for the root.
    pub fn file_name(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }

    /// The parent path (all components but the last), or `None` for the root. The parent of a top-level
    /// item is [`root`](RelPath::root).
    pub fn parent(&self) -> Option<RelPath> {
        if self.0.is_empty() {
            None
        } else {
            Some(RelPath(self.0[..self.0.len() - 1].to_vec()))
        }
    }

    /// Resolve this relative path against a filesystem `root`.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        for component in &self.0 {
            path.push(component);
        }
        path
    }
}

impl std::fmt::Display for RelPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            f.write_str(".")
        } else {
            f.write_str(&self.0.join("/"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_walks_up_one_level() {
        let p = RelPath::root().child("a").child("b").child("c.txt");
        assert_eq!(p.parent(), Some(RelPath::root().child("a").child("b")));
        // Parent of a top-level item is the root.
        assert_eq!(RelPath::root().child("top.txt").parent(), Some(RelPath::root()));
        // The root has no parent.
        assert_eq!(RelPath::root().parent(), None);
    }
}
