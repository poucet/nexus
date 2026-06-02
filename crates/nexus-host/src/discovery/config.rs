//! Optional user config that adds custom plugin directories.
//!
//! Plugins don't always live in the standard macOS folders — users keep them on
//! external drives or in personal directories. This module reads an optional
//! TOML config whose directories *extend* (never replace) the defaults from
//! [`super::paths`].
//!
//! Location: `$NEXUS_CONFIG` if set, otherwise
//! `~/Library/Application Support/Nexus/config.toml`. A missing or invalid
//! config is non-fatal — discovery falls back to the defaults.
//!
//! ```toml
//! [plugins]
//! clap = ["~/Plugins/CLAP", "/Volumes/Audio/CLAP"]
//! vst3 = ["/Volumes/Audio/VST3"]
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::PluginFormat;

/// Parsed Nexus config. Unknown keys are ignored so the file can carry settings
/// for other subsystems without breaking discovery.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    plugins: PluginDirs,
}

#[derive(Debug, Default, Deserialize)]
struct PluginDirs {
    #[serde(default)]
    clap: Vec<PathBuf>,
    #[serde(default)]
    vst3: Vec<PathBuf>,
}

impl Config {
    /// Extra search directories for `format`, in the order the user listed them.
    pub fn custom_dirs(&self, format: PluginFormat) -> &[PathBuf] {
        match format {
            PluginFormat::Clap => &self.plugins.clap,
            PluginFormat::Vst3 => &self.plugins.vst3,
        }
    }
}

/// Default config path: `$NEXUS_CONFIG`, else
/// `~/Library/Application Support/Nexus/config.toml`. `None` only if neither
/// `NEXUS_CONFIG` nor `HOME` is set.
pub fn config_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("NEXUS_CONFIG") {
        return Some(PathBuf::from(explicit));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Nexus/config.toml"))
}

/// Load the config from the default location, falling back to an empty config
/// when it is absent, unreadable, or malformed (the failure is logged).
pub fn load() -> Config {
    match config_path() {
        Some(path) => load_from(&path),
        None => Config::default(),
    }
}

/// Load the config from a specific path. A missing file yields the default
/// config; a malformed file is logged and also yields the default, so a typo
/// never stops the host from finding plugins in the standard folders.
pub fn load_from(path: &Path) -> Config {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return Config::default(), // absent config is the normal case
    };
    match toml::from_str(&text) {
        Ok(config) => config,
        Err(e) => {
            log::warn!("ignoring invalid Nexus config at {}: {e}", path.display());
            Config::default()
        }
    }
}

/// Expand a leading `~` or `~/…` to the given home directory. Other paths
/// (including `~user`, which we don't resolve) are returned unchanged.
pub fn expand_tilde(path: &Path, home: Option<&Path>) -> PathBuf {
    let (Some(home), Ok(rest)) = (home, path.strip_prefix("~")) else {
        return path.to_path_buf();
    };
    // `strip_prefix("~")` on "~/foo" leaves "foo"; on bare "~" leaves "".
    home.join(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_per_format_dirs() {
        let config: Config = toml::from_str(
            r#"
            [plugins]
            clap = ["/a/clap", "~/b/clap"]
            vst3 = ["/c/vst3"]
            "#,
        )
        .unwrap();
        assert_eq!(
            config.custom_dirs(PluginFormat::Clap),
            [PathBuf::from("/a/clap"), PathBuf::from("~/b/clap")]
        );
        assert_eq!(config.custom_dirs(PluginFormat::Vst3), [PathBuf::from("/c/vst3")]);
    }

    #[test]
    fn missing_sections_default_to_empty() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.custom_dirs(PluginFormat::Clap).is_empty());
        assert!(config.custom_dirs(PluginFormat::Vst3).is_empty());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let config: Config = toml::from_str(
            r#"
            unrelated = true
            [plugins]
            clap = ["/x"]
            "#,
        )
        .unwrap();
        assert_eq!(config.custom_dirs(PluginFormat::Clap), [PathBuf::from("/x")]);
    }

    #[test]
    fn missing_file_is_empty_config() {
        let config = load_from(Path::new("/no/such/nexus-config.toml"));
        assert!(config.custom_dirs(PluginFormat::Clap).is_empty());
    }

    #[test]
    fn tilde_expands_against_home() {
        let home = PathBuf::from("/Users/test");
        assert_eq!(
            expand_tilde(Path::new("~/Plugins/CLAP"), Some(&home)),
            PathBuf::from("/Users/test/Plugins/CLAP")
        );
        assert_eq!(expand_tilde(Path::new("~"), Some(&home)), home);
    }

    #[test]
    fn tilde_left_alone_without_home_or_prefix() {
        assert_eq!(expand_tilde(Path::new("~/x"), None), PathBuf::from("~/x"));
        assert_eq!(
            expand_tilde(Path::new("/abs/path"), Some(Path::new("/Users/test"))),
            PathBuf::from("/abs/path")
        );
    }
}
