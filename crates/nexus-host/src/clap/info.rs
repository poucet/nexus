//! Read-only informational extensions: latency and voice-info.
//!
//! `tail` is intentionally not here yet — CLAP queries it on the *audio*
//! processor handle, so it belongs with the audio half rather than the
//! main-thread instance. `note-name` (named notes / drum maps) is niche and
//! deferred to Phase 02.

use clack_extensions::latency::PluginLatency;
use clack_extensions::voice_info::PluginVoiceInfo;
use clack_host::prelude::*;

use crate::clap::host::NexusHost;
use crate::host::VoiceInfo;

/// The plugin's reported latency in samples (0 if it has no latency extension).
pub fn latency(instance: &mut PluginInstance<NexusHost>) -> u32 {
    let mut handle = instance.plugin_handle();
    let Some(latency) = handle.get_extension::<PluginLatency>() else {
        return 0;
    };
    latency.get(&mut handle)
}

/// The plugin's polyphony info, if it exposes `voice-info`.
pub fn voice_info(instance: &mut PluginInstance<NexusHost>) -> Option<VoiceInfo> {
    let mut handle = instance.plugin_handle();
    let voice_info = handle.get_extension::<PluginVoiceInfo>()?;
    let info = voice_info.get(&mut handle)?;
    Some(VoiceInfo {
        voice_count: info.voice_count,
        voice_capacity: info.voice_capacity,
    })
}

#[cfg(test)]
mod tests {
    use crate::{Host, PluginCatalog, PluginFormat};

    #[test]
    #[ignore = "requires the Surge XT CLAP plugin installed"]
    fn reads_latency_and_voice_info() {
        let catalog = PluginCatalog::scan();
        let Some(plugin) = catalog.plugins().iter().find(|p| {
            p.format == PluginFormat::Clap && p.descriptor.id == "org.surge-synth-team.surge-xt"
        }) else {
            eprintln!("Surge XT not found; skipping");
            return;
        };

        let host = Host::new();
        let mut instance = host.load(plugin).expect("load");
        println!("latency: {} samples", instance.latency());
        match instance.voice_info() {
            Some(vi) => {
                println!("voices: {}/{}", vi.voice_count, vi.voice_capacity);
                assert!(vi.voice_capacity >= vi.voice_count);
            }
            None => println!("no voice-info extension"),
        }
    }
}
