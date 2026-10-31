//! On-disk cache of probed plugin descriptors.
//!
//! Probing a CLAP bundle means loading its shared library and running its entry
//! initializer — which for some plugins (e.g. VCV Rack) is expensive and
//! side-effectful. The catalog caches each bundle's descriptors keyed by path
//! and modification time, so a launch only loads bundles that are new or have
//! changed since last time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::host::PluginDescriptor;

/// Cached probe results for one bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Bundle modification time (seconds since the Unix epoch).
    pub mtime: u64,
    pub descriptors: Vec<PluginDescriptor>,
    /// Why the probe failed, if it did. Cached so a broken bundle isn't
    /// re-loaded on every launch (and the failure can still be surfaced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The whole catalog cache: bundle path → its cached descriptors.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CatalogCache {
    pub entries: HashMap<PathBuf, CacheEntry>,
}

impl CatalogCache {
    /// Load the cache, returning an empty cache if it's missing or unreadable.
    pub fn load() -> Self {
        cache_path().map_or_else(Self::default, |path| Self::load_from(&path))
    }

    /// Load the cache at `path`, empty if it's missing or unreadable.
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(data) => serde_json::from_slice(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Write the cache (best-effort; failures are ignored).
    pub fn save(&self) {
        if let Some(path) = cache_path() {
            self.save_to(&path);
        }
    }

    /// Write the cache to `path` (best-effort; failures are ignored).
    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_vec(self) {
            let _ = std::fs::write(path, data);
        }
    }
}

/// Location of the cache file under the Nexus app-support directory.
pub fn cache_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Nexus/catalog-cache.json"))
}

/// Modification time of `path` in seconds since the Unix epoch, if available.
pub fn file_mtime(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_round_trips_and_old_caches_still_parse() {
        let entry = CacheEntry { mtime: 1, descriptors: vec![], error: Some("nope".into()) };
        let json = serde_json::to_string(&entry).unwrap();
        let back: CacheEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.error.as_deref(), Some("nope"));

        // Entries written before the `error` field existed still deserialize.
        let old: CacheEntry = serde_json::from_str(r#"{"mtime":1,"descriptors":[]}"#).unwrap();
        assert!(old.error.is_none());
    }
}
