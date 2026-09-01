//! Reading a CLAP plugin's parameters via its `params` extension.
//!
//! These are main-thread queries against the plugin's `PluginParams` extension.
//! Host-initiated parameter *changes* travel as `Event::ParamValue` in the
//! process event list (wired with the engine / `nexus-wrap`).

use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags, PluginParams};
use clack_host::prelude::*;

use crate::clap::host::NexusHost;
use crate::host::ParamInfo;

/// Enumerate the plugin's parameters. Empty if it has no `params` extension.
pub fn enumerate(instance: &mut PluginInstance<NexusHost>) -> Vec<ParamInfo> {
    let mut handle = instance.plugin_handle();
    let Some(params) = handle.get_extension::<PluginParams>() else {
        return Vec::new();
    };

    let mut buffer = ParamInfoBuffer::new();
    let count = params.count(&mut handle);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let Some(info) = params.get_info(&mut handle, i, &mut buffer) else {
            continue;
        };
        out.push(ParamInfo {
            id: info.id.get(),
            name: String::from_utf8_lossy(info.name).into_owned(),
            module: String::from_utf8_lossy(info.module).into_owned(),
            min_value: info.min_value,
            max_value: info.max_value,
            default_value: info.default_value,
            is_stepped: info.flags.contains(ParamInfoFlags::IS_STEPPED),
            is_readonly: info.flags.contains(ParamInfoFlags::IS_READONLY),
            is_hidden: info.flags.contains(ParamInfoFlags::IS_HIDDEN),
            is_bypass: info.flags.contains(ParamInfoFlags::IS_BYPASS),
            is_automatable: info.flags.contains(ParamInfoFlags::IS_AUTOMATABLE),
            is_modulatable: info.flags.contains(ParamInfoFlags::IS_MODULATABLE),
        });
    }
    out
}

/// The current plain value of parameter `id`, if available.
pub fn value(instance: &mut PluginInstance<NexusHost>, id: u32) -> Option<f64> {
    let mut handle = instance.plugin_handle();
    let params = handle.get_extension::<PluginParams>()?;
    let id = ClapId::from_raw(id)?;
    params.get_value(&mut handle, id)
}

/// Render `value` of parameter `id` as display text via the plugin.
pub fn value_to_text(
    instance: &mut PluginInstance<NexusHost>,
    id: u32,
    value: f64,
) -> Option<String> {
    let mut handle = instance.plugin_handle();
    let params = handle.get_extension::<PluginParams>()?;
    let id = ClapId::from_raw(id)?;
    let mut buffer = [0u8; 256];
    let written = params.value_to_text(&mut handle, id, value, &mut buffer).ok()?;
    Some(String::from_utf8_lossy(written).into_owned())
}

/// Parse display text into plain value for parameter `id` via the plugin.
pub fn text_to_value(
    instance: &mut PluginInstance<NexusHost>,
    id: u32,
    text: &str,
) -> Option<f64> {
    let mut handle = instance.plugin_handle();
    let params = handle.get_extension::<PluginParams>()?;
    let id = ClapId::from_raw(id)?;
    let c_str = std::ffi::CString::new(text).ok()?;
    params.text_to_value(&mut handle, id, &c_str)
}

#[cfg(test)]
mod tests {
    use crate::{Host, PluginCatalog, PluginFormat};

    #[test]
    #[ignore = "requires the Surge XT CLAP plugin installed"]
    fn enumerates_real_plugin_params() {
        let catalog = PluginCatalog::scan();
        let Some(plugin) = catalog.plugins().iter().find(|p| {
            p.format == PluginFormat::Clap && p.descriptor.id == "org.surge-synth-team.surge-xt"
        }) else {
            eprintln!("Surge XT not found; skipping");
            return;
        };

        let host = Host::new();
        let mut instance = host.load(plugin).expect("load");
        let params = instance.params();
        println!("Surge XT exposes {} parameters", params.len());
        for p in params.iter().take(5) {
            let text = instance.param_value_to_text(p.id, p.default_value);
            println!(
                "  '{}' [{}..{}] default {} -> {:?}",
                p.name, p.min_value, p.max_value, p.default_value, text
            );
        }
        assert!(!params.is_empty(), "expected the synth to expose parameters");
    }
}
