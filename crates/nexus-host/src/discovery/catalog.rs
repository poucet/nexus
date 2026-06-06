//! The plugin catalog: every installed plugin Nexus can offer.
//!
//! Built by scanning the search paths ([`super::scan`]) and probing each
//! bundle's factory for its descriptors. Bundles that fail to load are logged
//! and skipped — real plugin folders routinely contain broken or incompatible
//! binaries, and one bad bundle must never sink discovery.

use std::path::PathBuf;

use crate::PluginFormat;
use crate::discovery::{cache, scan};
use crate::host::{HostError, PluginDescriptor};
use crate::{clap, vst3};

/// Probe one bundle for its descriptors, dispatching on format.
fn probe_bundle(
    format: PluginFormat,
    path: &std::path::Path,
) -> Result<Vec<PluginDescriptor>, HostError> {
    match format {
        PluginFormat::Clap => clap::probe::probe(path),
        PluginFormat::Vst3 => vst3::probe::probe(path),
    }
}

/// A single plugin the host can load: its format, the bundle containing it, and
/// the descriptor read from that bundle's factory. One bundle may yield several
/// `PluginRef`s (a bundle can export multiple plugins).
#[derive(Debug, Clone)]
pub struct PluginRef {
    pub format: PluginFormat,
    pub path: PathBuf,
    pub descriptor: PluginDescriptor,
}

impl PluginRef {
    /// Best display name: the descriptor's name, falling back to its id.
    pub fn display_name(&self) -> &str {
        self.descriptor
            .name
            .as_deref()
            .unwrap_or(&self.descriptor.id)
    }
}

/// The full set of discovered plugins.
#[derive(Debug, Clone, Default)]
pub struct PluginCatalog {
    plugins: Vec<PluginRef>,
}

impl PluginCatalog {
    /// Scan the standard + configured directories and probe every bundle.
    ///
    /// Never fails: bundles that can't be loaded are logged at `warn` and
    /// skipped.
    pub fn scan() -> Self {
        let mut plugins = Vec::new();
        for (format, path) in scan::scan_all() {
            match probe_bundle(format, &path) {
                Ok(descriptors) => plugins.extend(descriptors.into_iter().map(|descriptor| {
                    PluginRef { format, path: path.clone(), descriptor }
                })),
                Err(e) => log::warn!("skipping plugin bundle: {e}"),
            }
        }
        Self { plugins }
    }

    /// Like [`scan`](Self::scan), but reuses an on-disk cache so unchanged
    /// bundles are **not** re-probed (and thus not loaded/initialized). Only new
    /// or modified bundles are probed; the cache is then rewritten.
    ///
    /// This is what apps should use at startup — a plain `scan()` loads every
    /// plugin's binary every time.
    pub fn load_or_scan() -> Self {
        let cached = cache::CatalogCache::load();
        let mut fresh = cache::CatalogCache::default();
        let mut plugins = Vec::new();

        for (format, path) in scan::scan_all() {
            let mtime = cache::file_mtime(&path);
            let descriptors = match (mtime, cached.entries.get(&path)) {
                // Unchanged since last probe — reuse, without loading the binary.
                (Some(m), Some(entry)) if entry.mtime == m => entry.descriptors.clone(),
                // New or changed — probe it (loads the binary).
                _ => match probe_bundle(format, &path) {
                    Ok(descriptors) => descriptors,
                    Err(e) => {
                        log::warn!("skipping plugin bundle: {e}");
                        continue;
                    }
                },
            };

            if let Some(m) = mtime {
                fresh.entries.insert(
                    path.clone(),
                    cache::CacheEntry { mtime: m, descriptors: descriptors.clone() },
                );
            }
            plugins.extend(descriptors.into_iter().map(|descriptor| PluginRef {
                format,
                path: path.clone(),
                descriptor,
            }));
        }

        fresh.save();
        Self { plugins }
    }

    pub fn plugins(&self) -> &[PluginRef] {
        &self.plugins
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_is_empty() {
        assert!(PluginCatalog::default().is_empty());
    }

    #[test]
    fn display_name_falls_back_to_id() {
        let named = PluginRef {
            format: PluginFormat::Clap,
            path: PathBuf::from("/x.clap"),
            descriptor: PluginDescriptor {
                id: "com.x.y".into(),
                name: Some("Pretty Name".into()),
                vendor: None,
                version: None,
            },
        };
        assert_eq!(named.display_name(), "Pretty Name");

        let unnamed = PluginRef {
            descriptor: PluginDescriptor {
                id: "com.x.y".into(),
                name: None,
                vendor: None,
                version: None,
            },
            ..named
        };
        assert_eq!(unnamed.display_name(), "com.x.y");
    }

    /// Build the catalog from the machine's real plugin folders. Ignored by
    /// default; run with `cargo test -- --ignored` to see the live list.
    #[test]
    #[ignore = "requires plugins installed on this machine"]
    fn scan_real_catalog() {
        let catalog = PluginCatalog::scan();
        for plugin in catalog.plugins() {
            println!(
                "{:?}  {}  ({})",
                plugin.format,
                plugin.display_name(),
                plugin.descriptor.id
            );
        }
        println!("catalog has {} plugin(s)", catalog.len());
        assert!(!catalog.is_empty(), "no plugins discovered");
    }
}
