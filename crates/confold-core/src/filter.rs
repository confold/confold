//! Include/exclude glob filtering.

use confold_vfs::RelPath;
use globset::{Glob, GlobSet, GlobSetBuilder};

/// A set of include/exclude globs applied to entries during comparison.
///
/// Globs match against both the full relative path (`a/b/c.tmp`) and the bare name (`c.tmp`). Excludes
/// apply to files and directories; includes apply to **files only**, so an include like `*.txt` does not
/// prune the directories needed to reach the matching files.
#[derive(Clone, Debug, Default)]
pub struct FilterSet {
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
}

impl FilterSet {
    /// Build a filter set from include and exclude glob patterns. Empty slices mean "no constraint".
    pub fn new(includes: &[String], excludes: &[String]) -> Result<Self, globset::Error> {
        Ok(FilterSet {
            include: build(includes)?,
            exclude: build(excludes)?,
        })
    }

    /// `true` if an entry at `rel` (a directory iff `is_dir`) should be skipped.
    pub fn is_excluded(&self, rel: &RelPath, is_dir: bool) -> bool {
        let path = rel.to_string();
        let name = rel.file_name().unwrap_or("");
        if let Some(exclude) = &self.exclude {
            if exclude.is_match(&path) || exclude.is_match(name) {
                return true;
            }
        }
        if !is_dir {
            if let Some(include) = &self.include {
                if !(include.is_match(&path) || include.is_match(name)) {
                    return true;
                }
            }
        }
        false
    }
}

fn build(globs: &[String]) -> Result<Option<GlobSet>, globset::Error> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        builder.add(Glob::new(glob)?);
    }
    Ok(Some(builder.build()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(parts: &[&str]) -> RelPath {
        let mut r = RelPath::root();
        for p in parts {
            r = r.child(p);
        }
        r
    }

    #[test]
    fn exclude_matches_name_and_path() {
        let f = FilterSet::new(&[], &["*.tmp".into(), "node_modules".into()]).unwrap();
        assert!(f.is_excluded(&rel(&["a", "b.tmp"]), false));
        assert!(f.is_excluded(&rel(&["node_modules"]), true));
        assert!(!f.is_excluded(&rel(&["a", "b.txt"]), false));
    }

    #[test]
    fn include_applies_to_files_not_dirs() {
        let f = FilterSet::new(&["*.txt".into()], &[]).unwrap();
        assert!(!f.is_excluded(&rel(&["sub"]), true)); // dir kept so we can reach files inside
        assert!(!f.is_excluded(&rel(&["a.txt"]), false));
        assert!(f.is_excluded(&rel(&["a.bin"]), false)); // non-matching file excluded
    }
}
