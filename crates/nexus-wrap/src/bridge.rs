//! Hosting bridge between nexus-wrap and the inner hosted plugin.
//!
//! Manages loading, parameter enumeration, state capture/restoration, and
//! offline audio rendering for the inner CLAP/VST3 plugin. Implements
//! [`MainThreadHost`] to connect directly to the [`nexus_tools`] subsystem.

use std::fs;

use nexus_host::discovery::catalog::{PluginCatalog, PluginRef};
use nexus_host::host::{AudioConfig, AudioProcessor, Host, HostError, ParamInfo, PluginDescriptor, PluginFormat, PluginInstance, PresetInfo, VoiceInfo};
use nexus_host::preset::Preset;
use nexus_host::render::render_offline;
use nexus_tools::render::{render_error, render_options, render_response};
use nexus_tools::bridge::{HostSession, MainThreadHost};
use nexus_tools::service::sha256_hex;
use nexus_tools::types::*;
use nexus_tools::ToolError;

fn format_name(format: PluginFormat) -> String {
    match format {
        PluginFormat::Clap => "clap".into(),
        PluginFormat::Vst3 => "vst3".into(),
    }
}

/// State of the inner hosted plugin, owned on the DAW's main thread.
pub struct InnerPluginState {
    pub host: Host,
    pub catalog: PluginCatalog,
    pub loaded: Option<PluginRef>,
    pub instance: Option<Box<dyn PluginInstance>>,
    pub sample_rate: Option<f64>,
    pub max_block_frames: usize,
}

impl InnerPluginState {
    /// The installed plugins (the cached standard catalog), no plugin loaded.
    pub fn new() -> Self {
        Self::with_catalog(PluginCatalog::load_or_scan())
    }

    /// Over a catalog the caller built: tests pass an empty one (or one from
    /// [`nexus_host::CatalogPaths`]) so no installed plugin is probed in the
    /// test process.
    pub fn with_catalog(catalog: PluginCatalog) -> Self {
        Self {
            host: Host::new(),
            catalog,
            loaded: None,
            instance: None,
            sample_rate: None,
            max_block_frames: 512,
        }
    }

    /// Activate the inner plugin's audio processor at the negotiated config.
    pub fn activate(&mut self, config: AudioConfig) -> Result<Box<dyn AudioProcessor>, HostError> {
        self.sample_rate = Some(config.sample_rate);
        self.max_block_frames = config.max_block_frames;
        if let Some(instance) = self.instance.as_mut() {
            instance.activate(config)
        } else {
            Err(HostError::Load("no inner plugin loaded".into()))
        }
    }

    /// Deactivate the inner plugin.
    pub fn deactivate(&mut self) {
        if let Some(instance) = self.instance.as_mut() {
            instance.deactivate();
        }
    }

    /// Query parameter info list from inner plugin.
    pub fn params(&mut self) -> Vec<ParamInfo> {
        self.instance.as_mut().map_or_else(Vec::new, |i| i.params())
    }

    /// Query plain value of parameter id from inner plugin.
    pub fn param_value(&mut self, id: u32) -> Option<f64> {
        self.instance.as_mut().and_then(|i| i.param_value(id))
    }

    /// Convert parameter value to display text via inner plugin.
    pub fn param_value_to_text(&mut self, id: u32, value: f64) -> Option<String> {
        self.instance.as_mut().and_then(|i| i.param_value_to_text(id, value))
    }

    /// Query presets list from inner plugin.
    pub fn presets(&mut self) -> Vec<PresetInfo> {
        self.instance.as_mut().map_or_else(Vec::new, |i| i.presets())
    }

    /// Query voice polyphony info from inner plugin.
    pub fn voice_info(&mut self) -> Option<VoiceInfo> {
        self.instance.as_mut().and_then(|i| i.voice_info())
    }

    /// Query latency in samples from inner plugin.
    pub fn latency(&mut self) -> u32 {
        self.instance.as_mut().map_or(0, |i| i.latency())
    }
}

impl Default for InnerPluginState {
    fn default() -> Self {
        Self::new()
    }
}

impl MainThreadHost for InnerPluginState {
    fn session(&mut self) -> HostSession {
        HostSession {
            loaded: self.loaded.as_ref().map(|p| LoadedPlugin {
                plugin_id: p.descriptor.id.clone(),
                name: p.descriptor.name.clone(),
                vendor: p.descriptor.vendor.clone(),
                format: format_name(p.format),
            }),
            sample_rate: self.sample_rate.or(Some(48000.0)),
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

        // Deactivate previous instance if any
        if let Some(mut prev) = self.instance.take() {
            prev.deactivate();
        }

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
        let plugin_ref = self.loaded.as_ref().ok_or_else(ToolError::no_plugin)?;
        let live_instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;

        // Capture live state so the render runs on a faithful clone and never
        // touches the live audio callback. This — and only this — is what
        // differs between the hosts; everything either side of `render_offline`
        // is `nexus_tools::render`.
        let state = live_instance.save_state().map_err(|e| ToolError::plugin(format!("save_state: {e}")))?;
        let mut clone = self.host.load(plugin_ref).map_err(|e| ToolError::plugin(format!("clone plugin instance: {e}")))?;
        let _ = clone.load_state(&state);

        let opts = render_options(req);
        let result = render_offline(&mut *clone, &opts).map_err(render_error)?;
        render_response(req, result)
    }
}
