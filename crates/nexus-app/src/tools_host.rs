//! `MainThreadHost` for the tools bridge: the app is the main-thread owner of
//! the `PluginInstance`, so tool requests (drained in `update()`) act on it here.

use nexus_host::PluginFormat;
use nexus_host::host::PluginDescriptor;
use nexus_tools::bridge::{HostSession, MainThreadHost};
use nexus_tools::types::LoadedPlugin;
use nexus_tools::ToolError;

use crate::app::NexusApp;

fn format_name(format: PluginFormat) -> String {
    match format {
        PluginFormat::Clap => "clap".into(),
        PluginFormat::Vst3 => "vst3".into(),
    }
}

impl MainThreadHost for NexusApp {
    fn session(&mut self) -> HostSession {
        HostSession {
            loaded: self.loaded.as_ref().map(|plugin| LoadedPlugin {
                plugin_id: plugin.descriptor.id.clone(),
                name: plugin.descriptor.name.clone(),
                vendor: plugin.descriptor.vendor.clone(),
                format: format_name(plugin.format),
            }),
            sample_rate: self.engine.as_ref().map(|engine| engine.sample_rate()),
        }
    }

    fn descriptor(&mut self) -> Option<PluginDescriptor> {
        self.loaded.as_ref().map(|plugin| plugin.descriptor.clone())
    }

    fn save_state(&mut self) -> Result<Vec<u8>, ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        instance.save_state().map_err(|e| ToolError::plugin(format!("save_state: {e}")))
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), ToolError> {
        let instance = self.instance.as_mut().ok_or_else(ToolError::no_plugin)?;
        instance.load_state(data).map_err(|e| ToolError::plugin(format!("load_state: {e}")))
    }
}
