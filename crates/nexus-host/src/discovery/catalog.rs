//! The plugin catalog: every installed plugin Nexus can offer.
//!
//! Built by scanning the search paths ([`super::scan`]) and probing each
//! bundle's factory for its descriptors. Bundles that fail to load are logged
//! and skipped — real plugin folders routinely contain broken or incompatible
//! binaries, and one bad bundle must never sink discovery.

use std::path::{Path, PathBuf};

use crate::PluginFormat;
use crate::discovery::{cache, config, paths, scan};
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

/// Where a catalog looks for bundles and where it caches their probes.
///
/// [`standard`](Self::standard) is what [`PluginCatalog::scan`] and
/// [`PluginCatalog::load_or_scan`] use: the platform folders plus the user's
/// Nexus config, cached under Nexus's app-support directory. A host that keeps
/// its own catalog (or a test that must load nothing) builds its own:
/// [`none`](Self::none) looks nowhere, [`with_dirs`](Self::with_dirs) looks
/// only where it is told, [`from_config`](Self::from_config) reads another
/// config file, and [`cache`](Self::cache) points the cache elsewhere (or
/// `None`: no cache is read or written).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogPaths {
    /// Directories searched for `.clap` bundles, in order.
    pub clap: Vec<PathBuf>,
    /// Directories searched for `.vst3` bundles, in order.
    pub vst3: Vec<PathBuf>,
    /// The probe cache [`PluginCatalog::load_or_scan_in`] reads and rewrites.
    pub cache: Option<PathBuf>,
}

impl CatalogPaths {
    /// The platform folders and the config at [`config::config_path`], cached
    /// at [`cache::cache_path`].
    pub fn standard() -> Self {
        Self::of(&config::load())
    }

    /// The platform folders and the config file at `path` (missing or invalid:
    /// the platform folders alone), cached at [`cache::cache_path`].
    pub fn from_config(path: &Path) -> Self {
        Self::of(&config::load_from(path))
    }

    fn of(config: &config::Config) -> Self {
        Self {
            clap: paths::search_paths_for(config, PluginFormat::Clap),
            vst3: paths::search_paths_for(config, PluginFormat::Vst3),
            cache: cache::cache_path(),
        }
    }

    /// Nowhere: a catalog built from it is empty and loads no plugin binary.
    pub fn none() -> Self {
        Self::default()
    }

    /// Only `clap` and `vst3`, with no cache.
    pub fn with_dirs(clap: Vec<PathBuf>, vst3: Vec<PathBuf>) -> Self {
        Self { clap, vst3, cache: None }
    }

    /// The same search directories, cached at `cache` (`None`: no cache).
    pub fn cache(self, cache: Option<PathBuf>) -> Self {
        Self { cache, ..self }
    }

    /// The search directories for `format`.
    pub fn dirs(&self, format: PluginFormat) -> &[PathBuf] {
        match format {
            PluginFormat::Clap => &self.clap,
            PluginFormat::Vst3 => &self.vst3,
        }
    }

    /// Every bundle under these directories, tagged with its format.
    fn bundles(&self) -> Vec<(PluginFormat, PathBuf)> {
        [PluginFormat::Clap, PluginFormat::Vst3]
            .into_iter()
            .flat_map(|format| {
                scan::scan_dirs(format, self.dirs(format))
                    .into_iter()
                    .map(move |path| (format, path))
            })
            .collect()
    }
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
        Self::scan_in(&CatalogPaths::standard())
    }

    /// [`scan`](Self::scan) over the directories in `paths` (its cache is
    /// neither read nor written).
    pub fn scan_in(paths: &CatalogPaths) -> Self {
        let mut catalog = Self::default();
        for (format, path) in paths.bundles() {
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
        Self::load_or_scan_in(&CatalogPaths::standard())
    }

    /// [`load_or_scan`](Self::load_or_scan) over the directories in `paths`,
    /// with its cache (no cache: every bundle is probed, nothing is written).
    pub fn load_or_scan_in(paths: &CatalogPaths) -> Self {
        let cached = paths
            .cache
            .as_deref()
            .map_or_else(cache::CatalogCache::default, cache::CatalogCache::load_from);
        let mut fresh = cache::CatalogCache::default();
        let mut catalog = Self::default();

        for (format, path) in paths.bundles() {
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

        if let Some(cache) = &paths.cache {
            fresh.save_to(cache);
        }
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

    /// A throwaway directory under the system temp dir, removed on drop.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("nexus-catalog-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_paths_is_an_empty_catalog_and_writes_no_cache() {
        let paths = CatalogPaths::none();
        assert!(PluginCatalog::scan_in(&paths).is_empty());
        let catalog = PluginCatalog::load_or_scan_in(&paths);
        assert!(catalog.is_empty() && catalog.failures().is_empty());
    }

    /// N13: the caller chooses the folders and the cache file. A bundle with no
    /// binary fails its probe, is recorded, and the cache lands where asked.
    #[test]
    fn caller_chosen_dirs_and_cache_are_the_only_ones_used() {
        let t = Temp::new("chosen");
        let clap = t.0.join("clap");
        std::fs::create_dir_all(clap.join("Empty.clap")).unwrap();
        let cache = t.0.join("cache/catalog.json");
        let paths = CatalogPaths::with_dirs(vec![clap.clone()], vec![]).cache(Some(cache.clone()));

        let catalog = PluginCatalog::load_or_scan_in(&paths);
        assert!(catalog.is_empty());
        assert_eq!(catalog.failures().len(), 1);
        assert_eq!(catalog.failures()[0].path, clap.join("Empty.clap"));

        let written = cache::CatalogCache::load_from(&cache);
        let entry = written.entries.get(&clap.join("Empty.clap")).expect("cached");
        assert!(entry.error.is_some());

        // A second load reuses the cached failure: same result, no re-probe.
        let again = PluginCatalog::load_or_scan_in(&paths);
        assert_eq!(again.failures()[0].error, catalog.failures()[0].error);
    }

    #[test]
    fn from_config_reads_the_given_file() {
        let t = Temp::new("config");
        let file = t.0.join("config.toml");
        std::fs::write(&file, "[plugins]\nvst3 = [\"/my/vst3\"]\n").unwrap();
        let paths = CatalogPaths::from_config(&file);
        assert_eq!(paths.dirs(PluginFormat::Vst3).last(), Some(&PathBuf::from("/my/vst3")));
        assert!(!paths.dirs(PluginFormat::Clap).contains(&PathBuf::from("/my/vst3")));
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
