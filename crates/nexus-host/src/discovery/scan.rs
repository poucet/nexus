//! Filesystem scan that locates plugin bundles under the search paths.
//!
//! This only *finds* bundles by their `.clap` / `.vst3` extension — it never
//! opens them. Reading a plugin's descriptor requires loading its binary and is
//! a later step ([`super`] catalog assembly).
//!
//! Implemented with plain recursive `read_dir` rather than `walkdir`: the one
//! awkward requirement is to yield a matched bundle *without descending into
//! it* (a `.vst3` bundle is itself a directory full of unrelated files), which
//! a hand-rolled walk expresses directly and without a dependency.

use std::fs;
use std::path::{Path, PathBuf};

use crate::PluginFormat;
use crate::discovery::paths;

/// Maximum directory depth to recurse while looking for bundles. Vendors nest
/// plugins a few folders deep; this also guards against symlink cycles.
const MAX_DEPTH: usize = 8;

impl PluginFormat {
    /// Bundle filename extension for this format, without the dot.
    fn bundle_extension(self) -> &'static str {
        match self {
            PluginFormat::Clap => "clap",
            PluginFormat::Vst3 => "vst3",
        }
    }
}

/// Locate every `format` bundle under the standard macOS search paths.
///
/// Returns absolute bundle paths (e.g. `.../CLAP/Foo.clap`). Missing or
/// unreadable search directories are skipped silently, and the scan never
/// descends into a matched bundle.
pub fn scan(format: PluginFormat) -> Vec<PathBuf> {
    scan_dirs(format, &paths::search_paths(format))
}

/// Locate every `format` bundle under `dirs`, in order. Like [`scan`], but the
/// caller chooses where to look.
pub fn scan_dirs(format: PluginFormat, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in dirs {
        scan_dir(dir, format, MAX_DEPTH, &mut found);
    }
    found
}

/// Scan every supported format and tag each bundle with its format.
pub fn scan_all() -> Vec<(PluginFormat, PathBuf)> {
    [PluginFormat::Clap, PluginFormat::Vst3]
        .into_iter()
        .flat_map(|format| scan(format).into_iter().map(move |path| (format, path)))
        .collect()
}

fn scan_dir(dir: &Path, format: PluginFormat, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return; // missing or unreadable — nothing to scan here
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if has_extension(&path, format.bundle_extension()) {
            out.push(path); // a bundle — record it, don't look inside
        } else if depth > 0 && path.is_dir() {
            scan_dir(&path, format, depth - 1, out); // recurse into vendor folders
        }
    }
}

/// Case-insensitive check that `path`'s final component ends in `.{ext}`.
fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A throwaway directory tree under the system temp dir, cleaned up on drop.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("nexus-scan-{}-{}", std::process::id(), n));
            fs::create_dir_all(&root).unwrap();
            TempTree(root)
        }

        /// Create a nested directory `rel` under the tree root.
        fn mkdir(&self, rel: &str) {
            fs::create_dir_all(self.0.join(rel)).unwrap();
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Sorted file names of the matched bundles, for stable assertions.
    fn names(paths: &[PathBuf]) -> Vec<String> {
        let mut v: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn scan_under(root: &Path, format: PluginFormat) -> Vec<PathBuf> {
        let mut out = Vec::new();
        scan_dir(root, format, MAX_DEPTH, &mut out);
        out
    }

    #[test]
    fn finds_bundles_including_nested_vendor_folders() {
        let t = TempTree::new();
        t.mkdir("Alpha.clap/Contents/MacOS");
        t.mkdir("Vendor/Beta.clap/Contents");
        t.mkdir("Other.vst3"); // wrong format — ignored when scanning for CLAP

        assert_eq!(names(&scan_under(&t.0, PluginFormat::Clap)), ["Alpha.clap", "Beta.clap"]);
    }

    #[test]
    fn does_not_descend_into_a_matched_bundle() {
        let t = TempTree::new();
        // A `.clap` nested *inside* a bundle must not be reported separately.
        t.mkdir("Outer.clap/Contents/Inner.clap");

        assert_eq!(names(&scan_under(&t.0, PluginFormat::Clap)), ["Outer.clap"]);
    }

    #[test]
    fn separates_formats() {
        let t = TempTree::new();
        t.mkdir("A.clap");
        t.mkdir("B.vst3");

        assert_eq!(names(&scan_under(&t.0, PluginFormat::Clap)), ["A.clap"]);
        assert_eq!(names(&scan_under(&t.0, PluginFormat::Vst3)), ["B.vst3"]);
    }

    #[test]
    fn missing_directory_yields_nothing() {
        let out = scan_under(Path::new("/no/such/dir/nexus-xyz"), PluginFormat::Clap);
        assert!(out.is_empty());
    }

    #[test]
    fn extension_match_is_case_insensitive_and_exact() {
        assert!(has_extension(Path::new("Foo.clap"), "clap"));
        assert!(has_extension(Path::new("Foo.CLAP"), "clap"));
        assert!(!has_extension(Path::new("Foo.vst3"), "clap"));
        assert!(!has_extension(Path::new("com.vendor"), "clap"));
    }
}
