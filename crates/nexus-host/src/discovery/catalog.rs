//! The plugin catalog: every installed plugin Nexus can offer.
//!
//! Built by scanning the search paths ([`super::scan`]) and probing each
//! bundle's factory for its descriptors. Bundles that fail to load are logged
//! and skipped — real plugin folders routinely contain broken or incompatible
//! binaries, and one bad bundle must never sink discovery.

use std::path::PathBuf;

use crate::PluginFormat;
use crate::clap;
use crate::discovery::scan;
use crate::host::PluginDescriptor;

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
    /// skipped. (VST3 probing arrives in T11; VST3 bundles are skipped until
    /// then.)
    pub fn scan() -> Self {
        let mut plugins = Vec::new();
        for (format, path) in scan::scan_all() {
            let descriptors = match format {
                PluginFormat::Clap => clap::probe::probe(&path),
                PluginFormat::Vst3 => continue, // TODO(T11): VST3 probe
            };
            match descriptors {
                Ok(descriptors) => plugins.extend(descriptors.into_iter().map(|descriptor| {
                    PluginRef { format, path: path.clone(), descriptor }
                })),
                Err(e) => log::warn!("skipping plugin bundle: {e}"),
            }
        }
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
