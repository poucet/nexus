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
use crate::discovery::config::{self, Config};

/// macOS subdirectory under `Library/Audio/Plug-Ins/` for each format.
impl PluginFormat {
    fn plugin_subdir(self) -> &'static str {
        match self {
            PluginFormat::Clap => "CLAP",
            PluginFormat::Vst3 => "VST3",
        }
    }
}

/// Search directories for `format`: the standard macOS locations followed by
/// any custom directories from the user's config (see [`config`]). Order is
/// most-specific first — user scope, system scope, then config dirs in the
/// order listed — with duplicates removed.
///
/// Directories are returned whether or not they exist on disk; locating them is
/// separate from scanning them, so callers decide how to treat a missing one.
pub fn search_paths(format: PluginFormat) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    search_paths_with(home.as_deref(), &config::load(), format)
}

/// Pure core of [`search_paths`] with the home directory and config injected,
/// so it can be unit-tested without touching the environment or filesystem.
fn search_paths_with(home: Option<&Path>, config: &Config, format: PluginFormat) -> Vec<PathBuf> {
    let mut paths = default_search_paths(home, format);
    for dir in config.custom_dirs(format) {
        paths.push(config::expand_tilde(dir, home));
    }
    dedup_preserving_order(paths)
}

/// The standard macOS directories for `format`, user scope before system scope.
/// A `None` home omits the user-scope directory.
fn default_search_paths(home: Option<&Path>, format: PluginFormat) -> Vec<PathBuf> {
    let subdir = format.plugin_subdir();
    let mut paths = Vec::with_capacity(2);
    if let Some(home) = home {
        paths.push(home.join("Library/Audio/Plug-Ins").join(subdir));
    }
    paths.push(Path::new("/Library/Audio/Plug-Ins").join(subdir));
    paths
}

/// Remove duplicate directories while keeping first-seen order (so defaults keep
/// precedence over a config dir that repeats them).
fn dedup_preserving_order(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    paths.into_iter().filter(|p| seen.insert(p.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(clap: &[&str], vst3: &[&str]) -> Config {
        let toml = format!(
            "[plugins]\nclap = [{}]\nvst3 = [{}]\n",
            clap.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>().join(", "),
            vst3.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>().join(", "),
        );
        toml::from_str(&toml).unwrap()
    }

    #[test]
    fn user_scope_comes_before_system_scope() {
        let home = PathBuf::from("/Users/test");
        let paths = default_search_paths(Some(&home), PluginFormat::Clap);
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
        let clap = default_search_paths(Some(&home), PluginFormat::Clap);
        let vst3 = default_search_paths(Some(&home), PluginFormat::Vst3);
        assert!(clap.iter().all(|p| p.ends_with("CLAP")));
        assert!(vst3.iter().all(|p| p.ends_with("VST3")));
    }

    #[test]
    fn missing_home_drops_user_scope() {
        let paths = default_search_paths(None, PluginFormat::Vst3);
        assert_eq!(paths, vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")]);
    }

    #[test]
    fn config_dirs_extend_defaults_with_tilde_expanded() {
        let home = PathBuf::from("/Users/test");
        let config = config_with(&["~/Extra/CLAP", "/Volumes/Audio/CLAP"], &[]);
        let paths = search_paths_with(Some(&home), &config, PluginFormat::Clap);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/test/Library/Audio/Plug-Ins/CLAP"),
                PathBuf::from("/Library/Audio/Plug-Ins/CLAP"),
                PathBuf::from("/Users/test/Extra/CLAP"),
                PathBuf::from("/Volumes/Audio/CLAP"),
            ]
        );
    }

    #[test]
    fn config_dir_duplicating_a_default_is_deduped() {
        let home = PathBuf::from("/Users/test");
        let config = config_with(&["~/Library/Audio/Plug-Ins/CLAP"], &[]);
        let paths = search_paths_with(Some(&home), &config, PluginFormat::Clap);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/test/Library/Audio/Plug-Ins/CLAP"),
                PathBuf::from("/Library/Audio/Plug-Ins/CLAP"),
            ]
        );
    }
}
