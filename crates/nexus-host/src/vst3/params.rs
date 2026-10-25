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

/// Plain → normalised for every parameter, sampled from the controller once,
/// on the thread that owns it, so the audio thread never calls the
/// `IEditController` (a UI-thread object) to convert a [`ParamChange`]'s plain
/// value (`I-100`).
///
/// Linear parameters (the plain range is `min + x·(max − min)`, which includes
/// every stepped one, and the identity most JUCE plugins use) convert exactly.
/// Any other curve is sampled at [`Normalizer::SAMPLES`] points and inverted
/// piecewise-linearly: exact at the samples, close between them, assuming the
/// curve is monotonic. A parameter the controller did not list is taken to be
/// normalised already.
///
/// [`ParamChange`]: crate::host::ParamChange
#[derive(Debug, Clone, Default)]
pub struct Normalizer {
    /// Sorted, for a binary search on the audio thread.
    ids: Vec<u32>,
    curves: Vec<Curve>,
}

#[derive(Debug, Clone, PartialEq)]
enum Curve {
    Linear { min: f64, max: f64 },
    /// `plain(k / (SAMPLES − 1))` for `k` in `0..SAMPLES`.
    Sampled(Box<[f64; Normalizer::SAMPLES]>),
}

impl Normalizer {
    /// Sample points for a curve that is not linear.
    pub const SAMPLES: usize = 33;

    /// Sample every parameter of `controller`. Main thread; not realtime.
    pub fn from_controller(controller: &ComPtr<IEditController>) -> Self {
        let count = unsafe { controller.getParameterCount() };
        let params = (0..count).filter_map(|index| {
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            (unsafe { controller.getParameterInfo(index, &mut info) } == kResultOk)
                .then_some((info.id, info.stepCount))
        });
        Self::sample(params, |id, x| unsafe { controller.normalizedParamToPlain(id, x) })
    }

    /// Build from `(id, step count)` pairs and the plugin's normalised → plain.
    pub fn sample(
        params: impl IntoIterator<Item = (u32, i32)>,
        plain: impl Fn(u32, f64) -> f64,
    ) -> Self {
        let mut entries: Vec<(u32, Curve)> = params
            .into_iter()
            .map(|(id, steps)| {
                let at = |k: usize| plain(id, k as f64 / (Self::SAMPLES - 1) as f64);
                let (min, max) = (at(0), at(Self::SAMPLES - 1));
                if steps > 0 {
                    // VST3 defines a discrete parameter as linear in its steps.
                    return (id, Curve::Linear { min, max });
                }
                let samples: [f64; Self::SAMPLES] = std::array::from_fn(at);
                let tolerance = 1e-9 * (max - min).abs().max(1.0);
                let linear = samples.iter().enumerate().all(|(k, &p)| {
                    let x = k as f64 / (Self::SAMPLES - 1) as f64;
                    (p - (min + x * (max - min))).abs() <= tolerance
                });
                let curve =
                    if linear { Curve::Linear { min, max } } else { Curve::Sampled(Box::new(samples)) };
                (id, curve)
            })
            .collect();
        entries.sort_by_key(|(id, _)| *id);
        entries.dedup_by_key(|(id, _)| *id);
        let (ids, curves) = entries.into_iter().unzip();
        Self { ids, curves }
    }

    /// Normalised (0..=1) value for plain `value` of parameter `id`.
    /// Realtime-safe: a binary search and arithmetic.
    pub fn normalize(&self, id: u32, value: f64) -> f64 {
        let Ok(i) = self.ids.binary_search(&id) else {
            return value.clamp(0.0, 1.0);
        };
        let x = match &self.curves[i] {
            Curve::Linear { min, max } if max != min => (value - min) / (max - min),
            Curve::Linear { .. } => 0.0,
            Curve::Sampled(samples) => invert(samples.as_slice(), value),
        };
        x.clamp(0.0, 1.0)
    }
}

/// Where `value` falls on a monotonic curve sampled evenly over 0..=1.
fn invert(samples: &[f64], value: f64) -> f64 {
    let last = samples.len() - 1;
    let rising = samples[last] >= samples[0];
    // First sample at or past `value` in the curve's direction.
    let k = samples.partition_point(|&p| if rising { p < value } else { p > value });
    if k == 0 {
        return 0.0;
    }
    if k > last {
        return 1.0;
    }
    let (a, b) = (samples[k - 1], samples[k]);
    let t = if b == a { 0.0 } else { (value - a) / (b - a) };
    (k - 1) as f64 / last as f64 + t / last as f64
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

#[cfg(test)]
mod tests {
    use super::Normalizer;

    #[test]
    fn linear_and_stepped_parameters_invert_exactly() {
        let n = Normalizer::sample([(1, 0), (2, 0), (3, 4)], |id, x| match id {
            1 => x,                              // identity (JUCE)
            2 => 20.0 + x * 19_980.0,            // a linear Hz range
            _ => (x * 4.0).round(),              // 5 discrete steps
        });
        assert_eq!(n.normalize(1, 0.3), 0.3);
        assert!((n.normalize(2, 10_010.0) - 0.5).abs() < 1e-12);
        assert_eq!(n.normalize(3, 1.0), 0.25);
        assert_eq!(n.normalize(3, 4.0), 1.0);
        assert_eq!(n.normalize(2, 1e9), 1.0, "clamped");
        assert_eq!(n.normalize(99, 0.7), 0.7, "an unlisted id is already normalised");
    }

    #[test]
    fn a_curved_parameter_inverts_through_its_samples() {
        // A log-style frequency curve, rising, and a falling one.
        let n = Normalizer::sample([(1, 0), (2, 0)], |id, x| match id {
            1 => 20.0 * 1000f64.powf(x),
            _ => 1.0 - x * x,
        });
        for x in [0.0, 0.1, 0.25, 0.5, 0.8, 1.0] {
            let up = n.normalize(1, 20.0 * 1000f64.powf(x));
            let down = n.normalize(2, 1.0 - x * x);
            assert!((up - x).abs() < 3e-3, "rising: {x} → {up}");
            assert!((down - x).abs() < 3e-3, "falling: {x} → {down}");
        }
        // Exact on a sample point.
        assert_eq!(n.normalize(1, 20.0 * 1000f64.powf(0.5)), 0.5);
    }
}
