//! Plugin search paths.
//!
//! These mirror the standard CLAP/VST3 locations that Bitwig, Reaper, Ableton, and
//! other hosts scan on macOS and Linux. Each format has a user-scope directory
//! and system-scope directories.

use std::path::{Path, PathBuf};

use crate::PluginFormat;
use crate::discovery::config::{self, Config};

/// Search directories for `format`: the standard platform locations followed by
/// any custom directories from the user's config (see [`config`]). Order is
/// most-specific first — user scope, system scope, then config dirs in the
/// order listed — with duplicates removed.
///
/// Directories are returned whether or not they exist on disk; locating them is
/// separate from scanning them, so callers decide how to treat a missing one.
pub fn search_paths(format: PluginFormat) -> Vec<PathBuf> {
    search_paths_for(&config::load(), format)
}

/// [`search_paths`] with the config the caller chose (say, one read with
/// [`config::load_from`]) instead of the one at [`config::config_path`].
pub fn search_paths_for(config: &Config, format: PluginFormat) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    search_paths_with(home.as_deref(), config, format)
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

/// The standard platform directories for `format`, user scope before system scope.
/// A `None` home omits the user-scope directory.
fn default_search_paths(home: Option<&Path>, format: PluginFormat) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos_default_search_paths(home, format)
    }
    #[cfg(target_os = "linux")]
    {
        linux_default_search_paths(home, format)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        linux_default_search_paths(home, format)
    }
}

/// The standard macOS directories for `format`, user scope before system scope.
#[allow(dead_code)]
pub(crate) fn macos_default_search_paths(home: Option<&Path>, format: PluginFormat) -> Vec<PathBuf> {
    let subdir = match format {
        PluginFormat::Clap => "CLAP",
        PluginFormat::Vst3 => "VST3",
    };
    let mut paths = Vec::with_capacity(2);
    if let Some(home) = home {
        paths.push(home.join("Library/Audio/Plug-Ins").join(subdir));
    }
    paths.push(Path::new("/Library/Audio/Plug-Ins").join(subdir));
    paths
}

/// The standard Linux directories for `format`, user scope before system scope:
/// - CLAP: User `~/.clap`, System `/usr/lib/clap`, `/usr/local/lib/clap`
/// - VST3: User `~/.vst3`, System `/usr/lib/vst3`, `/usr/local/lib/vst3`
#[allow(dead_code)]
pub(crate) fn linux_default_search_paths(home: Option<&Path>, format: PluginFormat) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(3);
    let (user_dir, sys_dir) = match format {
        PluginFormat::Clap => (".clap", "clap"),
        PluginFormat::Vst3 => (".vst3", "vst3"),
    };
    if let Some(home) = home {
        paths.push(home.join(user_dir));
    }
    paths.push(PathBuf::from(format!("/usr/lib/{sys_dir}")));
    paths.push(PathBuf::from(format!("/usr/local/lib/{sys_dir}")));
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
    fn macos_user_scope_comes_before_system_scope() {
        let home = PathBuf::from("/Users/test");
        let paths = macos_default_search_paths(Some(&home), PluginFormat::Clap);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/test/Library/Audio/Plug-Ins/CLAP"),
                PathBuf::from("/Library/Audio/Plug-Ins/CLAP"),
            ]
        );
    }

    #[test]
    fn macos_each_format_uses_its_own_subdir() {
        let home = PathBuf::from("/Users/test");
        let clap = macos_default_search_paths(Some(&home), PluginFormat::Clap);
        let vst3 = macos_default_search_paths(Some(&home), PluginFormat::Vst3);
        assert!(clap.iter().all(|p| p.ends_with("CLAP")));
        assert!(vst3.iter().all(|p| p.ends_with("VST3")));
    }

    #[test]
    fn macos_missing_home_drops_user_scope() {
        let paths = macos_default_search_paths(None, PluginFormat::Vst3);
        assert_eq!(paths, vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")]);
    }

    #[test]
    fn linux_clap_paths() {
        let home = PathBuf::from("/home/test");
        let paths = linux_default_search_paths(Some(&home), PluginFormat::Clap);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/home/test/.clap"),
                PathBuf::from("/usr/lib/clap"),
                PathBuf::from("/usr/local/lib/clap"),
            ]
        );
    }

    #[test]
    fn linux_vst3_paths() {
        let home = PathBuf::from("/home/test");
        let paths = linux_default_search_paths(Some(&home), PluginFormat::Vst3);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/home/test/.vst3"),
                PathBuf::from("/usr/lib/vst3"),
                PathBuf::from("/usr/local/lib/vst3"),
            ]
        );
    }

    #[test]
    fn linux_missing_home_drops_user_scope() {
        let paths = linux_default_search_paths(None, PluginFormat::Vst3);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/usr/lib/vst3"),
                PathBuf::from("/usr/local/lib/vst3"),
            ]
        );
    }

    #[test]
    fn config_dirs_extend_defaults_with_tilde_expanded() {
        let home = PathBuf::from("/Users/test");
        let config = config_with(&["~/Extra/CLAP", "/Volumes/Audio/CLAP"], &[]);
        let paths = search_paths_with(Some(&home), &config, PluginFormat::Clap);
        let defaults = default_search_paths(Some(&home), PluginFormat::Clap);
        let mut expected = defaults;
        expected.push(PathBuf::from("/Users/test/Extra/CLAP"));
        expected.push(PathBuf::from("/Volumes/Audio/CLAP"));
        assert_eq!(paths, expected);
    }

    #[test]
    fn config_dir_duplicating_a_default_is_deduped() {
        let home = PathBuf::from("/Users/test");
        let defaults = default_search_paths(Some(&home), PluginFormat::Clap);
        let first_default = defaults[0].to_str().unwrap();
        let config = config_with(&[first_default], &[]);
        let paths = search_paths_with(Some(&home), &config, PluginFormat::Clap);
        assert_eq!(paths, defaults);
    }
}
