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

/// A bundle discovery found but could not probe, kept so UIs can surface it
/// instead of the failure living only in the log.
#[derive(Debug, Clone)]
pub struct CatalogFailure {
    pub format: PluginFormat,
    pub path: PathBuf,
    pub error: String,
}

/// The full set of discovered plugins, plus the bundles that failed to probe.
#[derive(Debug, Clone, Default)]
pub struct PluginCatalog {
    plugins: Vec<PluginRef>,
    failures: Vec<CatalogFailure>,
}

impl PluginCatalog {
    fn push_plugins(
        &mut self,
        format: PluginFormat,
        path: &std::path::Path,
        descriptors: Vec<PluginDescriptor>,
    ) {
        self.plugins.extend(descriptors.into_iter().map(|descriptor| PluginRef {
            format,
            path: path.to_path_buf(),
            descriptor,
        }));
    }

    fn push_failure(&mut self, format: PluginFormat, path: &std::path::Path, error: String) {
        log::warn!("skipping plugin bundle {}: {error}", path.display());
        self.failures.push(CatalogFailure { format, path: path.to_path_buf(), error });
    }

    /// Scan the standard + configured directories and probe every bundle.
    ///
    /// Never fails: bundles that can't be loaded are recorded in
    /// [`failures`](Self::failures) (and logged) and skipped.
    pub fn scan() -> Self {
        let mut catalog = Self::default();
        for (format, path) in scan::scan_all() {
            match probe_bundle(format, &path) {
                Ok(descriptors) => catalog.push_plugins(format, &path, descriptors),
                Err(e) => catalog.push_failure(format, &path, e.to_string()),
            }
        }
        catalog
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
        let mut catalog = Self::default();

        for (format, path) in scan::scan_all() {
            let mtime = cache::file_mtime(&path);
            let (descriptors, error) = match (mtime, cached.entries.get(&path)) {
                // Unchanged since last probe — reuse, without loading the
                // binary (failed probes are cached too, so a broken bundle
                // isn't re-loaded every launch).
                (Some(m), Some(entry)) if entry.mtime == m => {
                    (entry.descriptors.clone(), entry.error.clone())
                }
                // New or changed — probe it (loads the binary).
                _ => match probe_bundle(format, &path) {
                    Ok(descriptors) => (descriptors, None),
                    Err(e) => (Vec::new(), Some(e.to_string())),
                },
            };

            if let Some(m) = mtime {
                fresh.entries.insert(
                    path.clone(),
                    cache::CacheEntry {
                        mtime: m,
                        descriptors: descriptors.clone(),
                        error: error.clone(),
                    },
                );
            }
            match error {
                Some(e) => catalog.push_failure(format, &path, e),
                None => catalog.push_plugins(format, &path, descriptors),
            }
        }

        fresh.save();
        catalog
    }

    pub fn plugins(&self) -> &[PluginRef] {
        &self.plugins
    }

    /// Bundles discovery found but could not probe.
    pub fn failures(&self) -> &[CatalogFailure] {
        &self.failures
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
    fn failures_are_recorded_not_fatal() {
        let mut catalog = PluginCatalog::default();
        catalog.push_failure(
            PluginFormat::Vst3,
            std::path::Path::new("/broken.vst3"),
            "boom".into(),
        );
        assert!(catalog.plugins().is_empty());
        assert_eq!(catalog.failures().len(), 1);
        assert_eq!(catalog.failures()[0].error, "boom");
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
