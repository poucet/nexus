//! VST3 parameters, read from the plugin's `IEditController`.
//!
//! VST3 parameters are normalized (0..1); we convert to/from each parameter's
//! plain range so the façade reports plain values consistently with CLAP.

use vst3_sdk::ComPtr;
use vst3_sdk::Steinberg::Vst::{
    IEditController, IEditControllerTrait, ParameterInfo, ParameterInfo_, String128, TChar,
};
use vst3_sdk::Steinberg::kResultOk;

use crate::host::ParamInfo;

/// Enumerate the controller's parameters.
pub fn enumerate(controller: &ComPtr<IEditController>) -> Vec<ParamInfo> {
    let count = unsafe { controller.getParameterCount() };
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for index in 0..count {
        let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
        if unsafe { controller.getParameterInfo(index, &mut info) } != kResultOk {
            continue;
        }
        let id = info.id;
        let flags = info.flags;
        let has = |flag: i32| flags & flag != 0;
        out.push(ParamInfo {
            id,
            name: string_from_utf16(&info.title),
            module: String::new(),
            min_value: unsafe { controller.normalizedParamToPlain(id, 0.0) },
            max_value: unsafe { controller.normalizedParamToPlain(id, 1.0) },
            default_value: unsafe {
                controller.normalizedParamToPlain(id, info.defaultNormalizedValue)
            },
            is_stepped: info.stepCount > 0,
            is_readonly: has(ParameterInfo_::ParameterFlags_::kIsReadOnly),
            is_hidden: has(ParameterInfo_::ParameterFlags_::kIsHidden),
            is_bypass: has(ParameterInfo_::ParameterFlags_::kIsBypass),
            is_automatable: has(ParameterInfo_::ParameterFlags_::kCanAutomate),
            // VST3 has no per-note modulation concept like CLAP.
            is_modulatable: false,
        });
    }
    out
}

/// Current plain value of parameter `id`.
pub fn value(controller: &ComPtr<IEditController>, id: u32) -> Option<f64> {
    let normalized = unsafe { controller.getParamNormalized(id) };
    Some(unsafe { controller.normalizedParamToPlain(id, normalized) })
}

/// Render plain `value` of parameter `id` as display text.
pub fn value_to_text(controller: &ComPtr<IEditController>, id: u32, value: f64) -> Option<String> {
    let normalized = unsafe { controller.plainParamToNormalized(id, value) };
    let mut buf: String128 = [0; 128];
    if unsafe { controller.getParamStringByValue(id, normalized, &mut buf) } == kResultOk {
        Some(string_from_utf16(&buf))
    } else {
        None
    }
}

/// Decode a null-terminated UTF-16 (`TChar`) buffer into a `String`.
fn string_from_utf16(buf: &[TChar]) -> String {
    let units: Vec<u16> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u16).collect();
    String::from_utf16_lossy(&units)
}

/// Parse display `text` into plain value for parameter `id`.
pub fn text_to_value(controller: &ComPtr<IEditController>, id: u32, text: &str) -> Option<f64> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let mut buf: String128 = [0; 128];
    for (dst, src) in buf.iter_mut().zip(utf16.iter()) {
        *dst = *src as TChar;
    }
    let mut norm: f64 = 0.0;
    if unsafe { controller.getParamValueByString(id, buf.as_mut_ptr(), &mut norm) } == kResultOk {
        Some(unsafe { controller.normalizedParamToPlain(id, norm) })
    } else {
        None
    }
}
