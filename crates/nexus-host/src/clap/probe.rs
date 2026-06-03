//! CLAP factory probe: read a bundle's plugin descriptors without instantiating.
//!
//! This loads the bundle's shared library and queries its `clap_plugin_factory`
//! for the list of plugins it provides, but never instantiates one. It feeds
//! the discovery catalog (T3).

use std::path::Path;

use clack_host::prelude::*;

use crate::host::{HostError, PluginDescriptor};

/// Read every plugin descriptor advertised by the `.clap` bundle at `path`.
///
/// Returns an error if the bundle can't be loaded or exposes no plugin factory.
/// Individual descriptors with an unreadable id are skipped.
pub fn probe(path: &Path) -> Result<Vec<PluginDescriptor>, HostError> {
    // SAFETY: loading a bundle runs its entry-point initializer. We only pass
    // paths found by scanning the standard plugin directories.
    let entry = unsafe { PluginEntry::load(path) }
        .map_err(|e| HostError::Load(format!("{}: {e}", path.display())))?;

    let factory = entry
        .get_plugin_factory()
        .ok_or_else(|| HostError::Load(format!("{}: no plugin factory", path.display())))?;

    let descriptors = factory
        .plugin_descriptors()
        .filter_map(|d| {
            Some(PluginDescriptor {
                id: d.id()?.to_str().ok()?.to_string(),
                name: d.name().map(|s| s.to_string_lossy().into_owned()),
                vendor: d.vendor().map(|s| s.to_string_lossy().into_owned()),
                version: d.version().map(|s| s.to_string_lossy().into_owned()),
            })
        })
        .collect();

    Ok(descriptors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probing_a_missing_bundle_errors() {
        let result = probe(Path::new("/no/such/nexus-plugin.clap"));
        assert!(matches!(result, Err(HostError::Load(_))));
    }

    /// Probe every CLAP plugin actually installed on this machine. Ignored by
    /// default (machine-dependent); run with `cargo test -- --ignored` to
    /// eyeball real discovery output.
    #[test]
    #[ignore = "requires CLAP plugins installed on this machine"]
    fn probe_installed_clap_plugins() {
        use crate::PluginFormat;
        use crate::discovery::scan;

        let mut total = 0;
        for path in scan::scan(PluginFormat::Clap) {
            match probe(&path) {
                Ok(descriptors) => {
                    for d in &descriptors {
                        println!("{} — {:?} by {:?}", d.id, d.name, d.vendor);
                        total += 1;
                    }
                }
                Err(e) => eprintln!("probe failed: {e}"),
            }
        }
        println!("probed {total} CLAP plugin descriptor(s)");
        assert!(total > 0, "no CLAP plugins found to probe");
    }
}
