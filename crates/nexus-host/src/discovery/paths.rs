//! macOS plugin search paths.
//!
//! These mirror the standard CLAP/VST3 locations that Bitwig, Ableton, and
//! other hosts scan on macOS. Each format has a user-scope directory under the
//! user's `~/Library` and a system-scope directory under `/Library`.
//!
//! This module is macOS-only for the 0.1 MVP; Windows and Linux search paths
//! are a later-phase feature.

use std::path::{Path, PathBuf};

use crate::PluginFormat;

/// macOS subdirectory under `Library/Audio/Plug-Ins/` for each format.
impl PluginFormat {
    fn plugin_subdir(self) -> &'static str {
        match self {
            PluginFormat::Clap => "CLAP",
            PluginFormat::Vst3 => "VST3",
        }
    }
}

/// Standard macOS search directories for `format`, most-specific first: the
/// user-scope directory before the system-scope one.
///
/// Directories are returned whether or not they exist on disk — locating them
/// is separate from scanning them, so callers decide how to treat a missing
/// directory.
pub fn search_paths(format: PluginFormat) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    search_paths_in(home.as_deref(), format)
}

/// Pure core of [`search_paths`] with the home directory injected, so it can be
/// unit-tested without touching the environment. A `None` home omits the
/// user-scope directory (leaving only the system-scope one).
fn search_paths_in(home: Option<&Path>, format: PluginFormat) -> Vec<PathBuf> {
    let subdir = format.plugin_subdir();
    let mut paths = Vec::with_capacity(2);
    if let Some(home) = home {
        paths.push(home.join("Library/Audio/Plug-Ins").join(subdir));
    }
    paths.push(Path::new("/Library/Audio/Plug-Ins").join(subdir));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_scope_comes_before_system_scope() {
        let home = PathBuf::from("/Users/test");
        let paths = search_paths_in(Some(&home), PluginFormat::Clap);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/test/Library/Audio/Plug-Ins/CLAP"),
                PathBuf::from("/Library/Audio/Plug-Ins/CLAP"),
            ]
        );
    }

    #[test]
    fn each_format_uses_its_own_subdir() {
        let home = PathBuf::from("/Users/test");
        let clap = search_paths_in(Some(&home), PluginFormat::Clap);
        let vst3 = search_paths_in(Some(&home), PluginFormat::Vst3);
        assert!(clap.iter().all(|p| p.ends_with("CLAP")));
        assert!(vst3.iter().all(|p| p.ends_with("VST3")));
    }

    #[test]
    fn missing_home_drops_user_scope() {
        let paths = search_paths_in(None, PluginFormat::Vst3);
        assert_eq!(paths, vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")]);
    }
}
