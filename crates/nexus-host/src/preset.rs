//! Presets: a saved reference to a plugin plus its state.
//!
//! A preset captures *which* plugin (format + factory id + bundle path) and the
//! plugin's opaque state chunk (from [`PluginInstance::save_state`]). Recalling
//! one re-loads that plugin via [`Host::load`] and restores its state — so a
//! preset round-trips a plugin's full configuration. No UI here; the wrapper UI
//! (T33) drives this.

use std::path::Path;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::host::{Host, HostError, PluginDescriptor, PluginFormat, PluginInstance};
use crate::discovery::catalog::PluginRef;

/// A saved plugin reference plus its serialized state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub format: PluginFormat,
    /// Factory id of the plugin within its bundle.
    pub plugin_id: String,
    /// Bundle path the plugin was loaded from.
    pub path: PathBuf,
    /// The plugin's opaque state chunk, base64-encoded on disk.
    #[serde(with = "base64_bytes")]
    pub state: Vec<u8>,
}

impl Preset {
    /// Capture a loaded plugin's current state as a preset.
    pub fn capture(plugin: &PluginRef, instance: &mut dyn PluginInstance) -> Result<Self, HostError> {
        Ok(Self {
            format: plugin.format,
            plugin_id: plugin.descriptor.id.clone(),
            path: plugin.path.clone(),
            state: instance.save_state()?,
        })
    }

    /// Write the preset to a `.nexuspreset` file (JSON; state base64-encoded).
    pub fn save_to_file(&self, path: &Path) -> Result<(), HostError> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| HostError::Preset(format!("serialize: {e}")))?;
        std::fs::write(path, json)
            .map_err(|e| HostError::Preset(format!("write {}: {e}", path.display())))
    }

    /// Read a preset from a `.nexuspreset` file.
    pub fn load_from_file(path: &Path) -> Result<Self, HostError> {
        let data = std::fs::read(path)
            .map_err(|e| HostError::Preset(format!("read {}: {e}", path.display())))?;
        serde_json::from_slice(&data).map_err(|e| HostError::Preset(format!("parse: {e}")))
    }

    /// Recall the preset: load the referenced plugin and restore its state.
    pub fn recall(&self, host: &Host) -> Result<Box<dyn PluginInstance>, HostError> {
        let plugin = PluginRef {
            format: self.format,
            path: self.path.clone(),
            descriptor: PluginDescriptor {
                id: self.plugin_id.clone(),
                name: None,
                vendor: None,
                version: None,
            },
        };
        let mut instance = host.load(&plugin)?;
        instance.load_state(&self.state)?;
        Ok(instance)
    }
}

/// Serde helper: store `Vec<u8>` as a base64 string rather than a number array.
mod base64_bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(deserializer)?;
        STANDARD.decode(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_path(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("nexus-preset-{}-{}-{}", std::process::id(), n, name))
    }

    #[test]
    fn preset_round_trips_through_a_file() {
        let preset = Preset {
            format: PluginFormat::Clap,
            plugin_id: "com.example.synth".into(),
            path: PathBuf::from("/Library/Audio/Plug-Ins/CLAP/Example.clap"),
            state: vec![0, 1, 2, 250, 255, 42],
        };

        let path = temp_path("round-trip.nexuspreset");
        preset.save_to_file(&path).expect("save");
        let loaded = Preset::load_from_file(&path).expect("load");
        let _ = std::fs::remove_file(&path);

        assert_eq!(preset, loaded);
    }

    #[test]
    fn state_is_base64_in_the_file_not_a_byte_array() {
        let preset = Preset {
            format: PluginFormat::Vst3,
            plugin_id: "x".into(),
            path: PathBuf::from("/x.vst3"),
            state: b"hello".to_vec(),
        };
        let path = temp_path("b64.nexuspreset");
        preset.save_to_file(&path).expect("save");
        let text = std::fs::read_to_string(&path).expect("read");
        let _ = std::fs::remove_file(&path);

        // "hello" base64 == "aGVsbG8=" — and definitely not a JSON array.
        assert!(text.contains("aGVsbG8="), "state should be base64: {text}");
        assert!(!text.contains("[104"), "state should not be a byte array");
    }

    #[test]
    #[ignore = "requires the Surge XT CLAP plugin installed"]
    fn captures_and_recalls_a_real_plugin() {
        use crate::{PluginCatalog, PluginFormat};

        let catalog = PluginCatalog::scan();
        let Some(plugin) = catalog.plugins().iter().find(|p| {
            p.format == PluginFormat::Clap && p.descriptor.id == "org.surge-synth-team.surge-xt"
        }) else {
            eprintln!("Surge XT not found; skipping");
            return;
        };

        let host = Host::new();
        let mut instance = host.load(plugin).expect("load");
        let preset = Preset::capture(plugin, instance.as_mut()).expect("capture");
        println!("captured preset: {} bytes of state", preset.state.len());

        let path = temp_path("surge.nexuspreset");
        preset.save_to_file(&path).expect("save");
        let loaded = Preset::load_from_file(&path).expect("load");
        let _ = std::fs::remove_file(&path);

        let _recalled = loaded.recall(&host).expect("recall");
        println!("recalled plugin from preset OK");
    }
}
