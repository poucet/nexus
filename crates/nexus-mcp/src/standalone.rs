//! Standalone headless host implementation of [`MainThreadHost`] for `nexus-mcp`.
//! Allows AI agents to use all Nexus capabilities (discovery, loading, state, sampling)
//! completely headless without the GUI app.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use nexus_host::discovery::catalog::{PluginCatalog, PluginRef};
use nexus_host::host::PluginDescriptor;
use nexus_host::render::{render_offline, RenderOptions};
use nexus_host::{Host, PluginFormat, PluginInstance, Preset};
use nexus_tools::bridge::{HostBridge, HostRequest, HostSession, MainThreadHost};
use nexus_tools::service::sha256_hex;
use nexus_tools::types::*;
use nexus_tools::{ToolError, ToolRegistry, ToolsService};

fn format_name(format: PluginFormat) -> String {
    match format {
        PluginFormat::Clap => "clap".into(),
        PluginFormat::Vst3 => "vst3".into(),
    }
}

pub struct StandaloneHost {
    host: Host,
    catalog: PluginCatalog,
    loaded: Option<PluginRef>,
    instance: Option<Box<dyn PluginInstance>>,
}

impl StandaloneHost {
    pub fn new() -> Self {
        let catalog = PluginCatalog::load_or_scan();
        Self {
            host: Host::new(),
            catalog,
            loaded: None,
            instance: None,
        }
    }
}

impl Default for StandaloneHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MainThreadHost for StandaloneHost {
    fn session(&mut self) -> HostSession {
        HostSession {
            loaded: self.loaded.as_ref().map(|p| LoadedPlugin {
                plugin_id: p.descriptor.id.clone(),
                name: p.descriptor.name.clone(),
                vendor: p.descriptor.vendor.clone(),
                format: format_name(p.format),
            }),
            sample_rate: Some(48000.0),
        }
    }

    fn descriptor(&mut self) -> Option<PluginDescriptor> {
        self.loaded.as_ref().map(|p| p.descriptor.clone())
    }

    fn save_state(&mut self) -> Result<Vec<u8>, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        instance.save_state().map_err(|e| ToolError::plugin(format!("save_state: {e}")))
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        instance.load_state(data).map_err(|e| ToolError::plugin(format!("load_state: {e}")))
    }

    fn list_plugins(&mut self, query: Option<&str>) -> Result<Vec<PluginItem>, ToolError> {
        let q = query.map(|s| s.to_lowercase());
        let mut results = Vec::new();

        for p in self.catalog.plugins() {
            let matches = match &q {
                Some(query_str) => {
                    p.descriptor.id.to_lowercase().contains(query_str)
                        || p.display_name().to_lowercase().contains(query_str)
                        || p.descriptor.vendor.as_deref().is_some_and(|v| v.to_lowercase().contains(query_str))
                }
                None => true,
            };

            if matches {
                results.push(PluginItem {
                    id: p.descriptor.id.clone(),
                    name: p.descriptor.name.clone(),
                    vendor: p.descriptor.vendor.clone(),
                    version: p.descriptor.version.clone(),
                    format: format_name(p.format),
                    path: p.path.display().to_string(),
                });
            }
        }

        results.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(results)
    }

    fn load_plugin(&mut self, target: &str) -> Result<LoadedPlugin, ToolError> {
        let target_lower = target.to_lowercase();
        let plugins = self.catalog.plugins();

        let found = plugins
            .iter()
            .find(|p| p.descriptor.id == target)
            .or_else(|| plugins.iter().find(|p| p.display_name().eq_ignore_ascii_case(target)))
            .or_else(|| plugins.iter().find(|p| p.descriptor.id.to_lowercase().contains(&target_lower)))
            .or_else(|| plugins.iter().find(|p| p.display_name().to_lowercase().contains(&target_lower)))
            .cloned();

        let plugin_ref = found.ok_or_else(|| {
            ToolError::not_found(format!("plugin {:?} not found in catalog ({} available)", target, plugins.len()))
        })?;

        let instance = self.host.load(&plugin_ref).map_err(|e| ToolError::plugin(format!("load plugin: {e}")))?;

        let loaded = LoadedPlugin {
            plugin_id: plugin_ref.descriptor.id.clone(),
            name: plugin_ref.descriptor.name.clone(),
            vendor: plugin_ref.descriptor.vendor.clone(),
            format: format_name(plugin_ref.format),
        };

        self.instance = Some(instance);
        self.loaded = Some(plugin_ref);
        Ok(loaded)
    }

    fn list_presets(&mut self, _plugin_id_filter: Option<&str>) -> Result<Vec<PresetItem>, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        let raw = instance.presets();
        Ok(raw
            .into_iter()
            .map(|p| PresetItem {
                index: p.index,
                name: p.name,
                category: p.category,
                program_list_id: p.program_list_id,
            })
            .collect())
    }

    fn load_preset(&mut self, preset_name_or_index: &str) -> Result<LoadPresetResponse, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        instance
            .load_preset(preset_name_or_index)
            .map_err(|e| ToolError::plugin(format!("load preset: {e}")))?;

        let presets = instance.presets();
        let (name, index) = if let Ok(idx) = preset_name_or_index.parse::<u32>() {
            let n = presets
                .iter()
                .find(|p| p.index == idx)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| preset_name_or_index.to_string());
            (n, idx)
        } else {
            let idx = presets
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(preset_name_or_index))
                .map(|p| p.index)
                .unwrap_or(0);
            (preset_name_or_index.to_string(), idx)
        };

        Ok(LoadPresetResponse {
            name,
            index,
            loaded: true,
        })
    }

    fn save_preset(&mut self, name: &str, _description: Option<&str>) -> Result<SavePresetResponse, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        let loaded = self.loaded.as_ref().ok_or_else(ToolError::no_plugin)?;

        let preset = Preset::capture(loaded, instance.as_mut())
            .map_err(|e| ToolError::plugin(format!("capture preset: {e}")))?;

        let dir = nexus_host::discovery::config::preset_directory();
        if let Err(e) = fs::create_dir_all(&dir) {
            return Err(ToolError::internal(format!("create preset dir: {e}")));
        }

        let safe_name = name.replace(['/', '\\', ':'], "_");
        let path = dir.join(format!("{safe_name}.nexuspreset"));
        preset.save_to_file(&path).map_err(|e| ToolError::internal(format!("save preset: {e}")))?;

        Ok(SavePresetResponse {
            name: name.to_string(),
            path: path.display().to_string(),
            size_bytes: preset.state.len() as u64,
            sha256: sha256_hex(&preset.state),
        })
    }

    fn list_params(
        &mut self,
        query: Option<&str>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<ListParamsResponse, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        let all_params = instance.params();

        let q = query.map(|s| s.to_lowercase());
        let filtered: Vec<_> = all_params
            .into_iter()
            .filter(|p| {
                match &q {
                    Some(qs) => p.name.to_lowercase().contains(qs) || p.module.to_lowercase().contains(qs),
                    None => true,
                }
            })
            .collect();

        let total = filtered.len();
        let off = offset.unwrap_or(0);
        let lim = limit.unwrap_or(100);

        let slice = filtered.into_iter().skip(off).take(lim);
        let mut params = Vec::new();

        for p in slice {
            let current_value = instance.param_value(p.id);
            let display_text = current_value.and_then(|v| instance.param_value_to_text(p.id, v));
            params.push(ParamItem {
                id: p.id,
                name: p.name,
                module: p.module,
                min_value: p.min_value,
                max_value: p.max_value,
                default_value: p.default_value,
                current_value,
                display_text,
                is_stepped: p.is_stepped,
                is_readonly: p.is_readonly,
            });
        }

        Ok(ListParamsResponse { total, offset: off, params })
    }

    fn get_params(&mut self, ids: &[u32]) -> Result<Vec<ParamValueItem>, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        let mut results = Vec::new();

        for &id in ids {
            if let Some(val) = instance.param_value(id) {
                let display_text = instance.param_value_to_text(id, val);
                results.push(ParamValueItem { id, value: val, display_text });
            }
        }

        Ok(results)
    }

    fn set_params(&mut self, params: &[SetParamItem]) -> Result<Vec<ParamValueItem>, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        let mut updated = Vec::new();

        for item in params {
            let _ = instance.set_param(item.id, item.value);
            if let Some(val) = instance.param_value(item.id) {
                let display_text = instance.param_value_to_text(item.id, val);
                updated.push(ParamValueItem { id: item.id, value: val, display_text });
            } else {
                let display_text = instance.param_value_to_text(item.id, item.value);
                updated.push(ParamValueItem { id: item.id, value: item.value, display_text });
            }
        }

        Ok(updated)
    }

    fn convert_param_value(
        &mut self,
        id: u32,
        value: Option<f64>,
        text: Option<&str>,
    ) -> Result<ConvertParamValueResponse, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        let mut out_val = value;
        let mut out_text = None;

        if let Some(t) = text {
            if let Some(v) = instance.param_text_to_value(id, t) {
                out_val = Some(v);
                out_text = Some(t.to_string());
            }
        } else if let Some(v) = value {
            out_text = instance.param_value_to_text(id, v);
        }

        Ok(ConvertParamValueResponse {
            id,
            value: out_val,
            display_text: out_text,
        })
    }

    fn play(&mut self, fugue: &nexus_host::fugue::CompositeFugue) -> Result<PlayResponse, ToolError> {
        let compiled = fugue.compile();
        Ok(PlayResponse {
            queued: true,
            duration_beats: compiled.duration_beats,
            total_events: compiled.events.len(),
        })
    }

    fn render_audio(&mut self, req: &RenderAudioRequest) -> Result<RenderAudioResponse, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;

        let compiled_fugue = req.fugue.compile();
        let opts = RenderOptions {
            sample_rate: req.sample_rate.unwrap_or(48000) as f64,
            block_size: req.block_size.unwrap_or(512),
            tempo_bpm: req.tempo_bpm.unwrap_or(120.0),
            fugue: compiled_fugue,
        };

        let result = render_offline(instance.as_mut(), &opts).map_err(|e| ToolError::plugin(format!("render_offline: {e}")))?;

        let format = req.output_format.as_deref().unwrap_or("wav_base64");
        let (wav_base64, file_path) = match format {
            "file" => {
                let out_path = if let Some(custom) = &req.output_path {
                    PathBuf::from(custom)
                } else {
                    let dir = nexus_host::discovery::config::preset_directory().join("rendered");
                    let _ = fs::create_dir_all(&dir);
                    dir.join(format!("sample_{}.wav", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()))
                };
                if let Err(e) = fs::write(&out_path, &result.wav_bytes) {
                    return Err(ToolError::internal(format!("write rendered wav: {e}")));
                }
                (None, Some(out_path.display().to_string()))
            }
            _ => {
                let b64 = BASE64.encode(&result.wav_bytes);
                (Some(b64), None)
            }
        };

        Ok(RenderAudioResponse {
            sample_rate: result.sample_rate,
            channels: result.channels.len(),
            frames: result.frames,
            duration_s: result.duration_s,
            peak_db: result.peak_db,
            rms_db: result.rms_db,
            is_silent: result.is_silent,
            wav_base64,
            file_path,
        })
    }
}

pub fn create_standalone_registry() -> (Arc<ToolRegistry>, crossbeam_channel::Receiver<HostRequest>) {
    let (bridge, rx) = HostBridge::channel(Arc::new(|| {}));
    let service = Arc::new(ToolsService::new(bridge));
    let registry = Arc::new(ToolRegistry::standard(service));
    (registry, rx)
}

pub fn spawn_standalone_worker(rx: crossbeam_channel::Receiver<HostRequest>) {
    std::thread::Builder::new()
        .name("nexus-standalone-worker".into())
        .spawn(move || {
            let mut host = StandaloneHost::new();
            while let Ok(req) = rx.recv() {
                nexus_tools::bridge::handle(req, &mut host);
            }
        })
        .expect("spawn standalone worker thread");
}
