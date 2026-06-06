//! Enumerating a VST3 bundle's audio plugin classes (no instantiation).

use std::path::Path;

use vst3_sdk::Steinberg::{IPluginFactoryTrait, PClassInfo, PFactoryInfo, TUID, kResultOk};

use crate::host::{HostError, PluginDescriptor};
use crate::vst3::module::Vst3Module;

/// VST3 category for audio effect / instrument classes.
const AUDIO_MODULE_CLASS: &str = "Audio Module Class";

/// Read the audio-plugin descriptors a `.vst3` bundle exposes, without
/// instantiating anything. The plugin id is the class UID (hex-encoded).
pub fn probe(bundle: &Path) -> Result<Vec<PluginDescriptor>, HostError> {
    let module = Vst3Module::load(bundle)?;
    let factory = &module.factory;

    // Vendor is factory-wide.
    let vendor = unsafe {
        let mut info: PFactoryInfo = std::mem::zeroed();
        if factory.getFactoryInfo(&mut info) == kResultOk {
            let vendor = char8_to_string(&info.vendor);
            (!vendor.is_empty()).then_some(vendor)
        } else {
            None
        }
    };

    let mut descriptors = Vec::new();
    let count = unsafe { factory.countClasses() };
    for index in 0..count {
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        if unsafe { factory.getClassInfo(index, &mut info) } != kResultOk {
            continue;
        }
        if char8_to_string(&info.category) != AUDIO_MODULE_CLASS {
            continue;
        }
        descriptors.push(PluginDescriptor {
            id: cid_to_hex(&info.cid),
            name: Some(char8_to_string(&info.name)).filter(|s| !s.is_empty()),
            vendor: vendor.clone(),
            version: None,
        });
    }
    Ok(descriptors)
}

/// Decode a null-terminated `char8` (i8) buffer into a `String`.
fn char8_to_string(buf: &[i8]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Hex-encode a class UID so it round-trips as a plugin id.
fn cid_to_hex(cid: &TUID) -> String {
    cid.iter().map(|&b| format!("{:02x}", b as u8)).collect()
}
