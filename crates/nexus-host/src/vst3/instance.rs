//! Instantiating and activating a VST3 plugin behind the host façade.
//!
//! [`Vst3PluginInstance`] is the main-thread half (implements
//! [`crate::host::PluginInstance`]); [`activate`](Vst3PluginInstance::activate)
//! sets up the audio buses and produces the `Send`
//! [`Vst3AudioProcessor`](crate::vst3::process::Vst3AudioProcessor) the engine
//! drives on the realtime thread.

use std::path::Path;

use vst3_sdk::Steinberg::Vst::{
    BusDirection, BusDirections_, BusInfo, IAudioProcessor, IAudioProcessorTrait, IComponent,
    IComponentTrait, IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait,
    IUnitInfoTrait, MediaType, MediaTypes_, ProcessModes_, ProcessSetup, SpeakerArr,
    SpeakerArrangement, SymbolicSampleSizes_,
};
use vst3_sdk::Steinberg::{FUnknown, IBStream, IPluginBaseTrait, TUID, kResultOk};
use vst3_sdk::{ComPtr, ComWrapper};

use crate::host::{AudioConfig, AudioProcessor, EventSupport, HostError, PluginInstance};
#[cfg(target_os = "macos")]
use crate::vst3::editor::Vst3Editor;
use crate::vst3::host_app::Vst3HostApp;
use crate::vst3::module::{Vst3Factory, Vst3Module};
use crate::vst3::process::Vst3AudioProcessor;
use crate::vst3::state::HostStream;

/// A loaded VST3 plugin (main-thread side).
pub struct Vst3PluginInstance {
    /// The plugin's editor, if open. Declared first so its view is released
    /// before the component/controller it came from.
    #[cfg(target_os = "macos")]
    editor: Option<Vst3Editor>,
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    /// The host context the plugin holds a reference to; kept alive here.
    _host: ComWrapper<Vst3HostApp>,
    active: bool,
    /// Owns the factory + dylib; dropped last so released COM objects above
    /// always call into a still-loaded library.
    module: Vst3Module,
    /// Parameter change ring producer, created on activate.
    param_producer: Option<rtrb::Producer<crate::host::ParamChange>>,
}

impl Vst3PluginInstance {
    /// Load the `.vst3` bundle at `path` and instantiate the audio class whose
    /// hex-encoded class UID is `plugin_id` (as produced by the probe).
    pub fn load(path: &Path, plugin_id: &str) -> Result<Self, HostError> {
        let module = Vst3Factory::load(path)?.host();
        let cid = parse_cid(plugin_id)
            .ok_or_else(|| HostError::Load(format!("invalid VST3 class id: {plugin_id}")))?;

        let host = Vst3HostApp::new();
        let ctx = host
            .to_com_ptr::<FUnknown>()
            .ok_or_else(|| HostError::Load("could not create host context".into()))?;

        // Instantiate the component and bring it up.
        let component: ComPtr<IComponent> = module
            .create_instance(&cid)
            .ok_or_else(|| HostError::Load("createInstance(IComponent) failed".into()))?;
        unsafe { component.initialize(ctx.as_ptr()) };

        let processor = component
            .cast::<IAudioProcessor>()
            .ok_or_else(|| HostError::Load("plugin has no IAudioProcessor".into()))?;

        // The edit controller may be the component itself, or a separate class.
        let controller = unsafe { setup_controller(&module, &component, ctx.as_ptr()) };

        Ok(Self {
            #[cfg(target_os = "macos")]
            editor: None,
            component,
            processor,
            controller,
            _host: host,
            active: false,
            module,
            param_producer: None,
        })
    }

    fn teardown(&mut self) {
        self.close_editor();
        if self.active {
            unsafe { self.component.setActive(0) };
            self.active = false;
        }
        self.param_producer = None;
        unsafe {
            if let Some(controller) = &self.controller {
                controller.terminate();
            }
            self.component.terminate();
        }
    }
}

impl PluginInstance for Vst3PluginInstance {
    /// **Neither, and that is deliberate rather than forgotten.** `to_vst_event`
    /// (`vst3/process.rs`) converts note on/off and drops every other raw-MIDI
    /// status — CC, pitch bend, channel and poly pressure — and builds no
    /// `kNoteExpressionValueEvent`, though the VST3 bindings have one. Saying so
    /// here is what turns a silent drop in the offline renderer into a warning
    /// the caller can read.
    fn event_support(&self) -> EventSupport {
        EventSupport { raw_midi: false, note_expression: false }
    }

    fn activate(&mut self, config: AudioConfig) -> Result<Box<dyn AudioProcessor>, HostError> {
        let component = &self.component;
        let processor = &self.processor;
        let audio = MediaTypes_::kAudio as MediaType;
        let event = MediaTypes_::kEvent as MediaType;
        let input = BusDirections_::kInput as BusDirection;
        let output = BusDirections_::kOutput as BusDirection;

        let in_buses = unsafe { component.getBusCount(audio, input) }.max(0);
        let out_buses = unsafe { component.getBusCount(audio, output) }.max(0);
        // Channel count of every audio bus (in bus-index order, bus 0 = main).
        let in_channels: Vec<usize> =
            (0..in_buses).map(|i| unsafe { bus_channels(component, input, i) }).collect();
        let out_channels: Vec<usize> =
            (0..out_buses).map(|i| unsafe { bus_channels(component, output, i) }).collect();

        // Negotiate speaker arrangements (best effort — keep going if rejected).
        let mut in_arr: Vec<SpeakerArrangement> =
            (0..in_buses).map(|i| unsafe { arrangement(component, input, i) }).collect();
        let mut out_arr: Vec<SpeakerArrangement> =
            (0..out_buses).map(|i| unsafe { arrangement(component, output, i) }).collect();
        // Best effort — keep going even if the plugin rejects the arrangement.
        unsafe {
            processor.setBusArrangements(
                in_arr.as_mut_ptr(),
                in_buses,
                out_arr.as_mut_ptr(),
                out_buses,
            );
        }

        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            maxSamplesPerBlock: config.max_block_frames as i32,
            sampleRate: config.sample_rate,
        };
        unsafe { processor.setupProcessing(&mut setup) };

        // Activate the audio buses we use, plus any event-input bus (needed for
        // instruments to receive notes).
        for i in 0..in_buses {
            unsafe { component.activateBus(audio, input, i, 1) };
        }
        for i in 0..out_buses {
            unsafe { component.activateBus(audio, output, i, 1) };
        }
        let event_in = unsafe { component.getBusCount(event, input) }.max(0);
        for i in 0..event_in {
            unsafe { component.activateBus(event, input, i, 1) };
        }

        unsafe { component.setActive(1) };
        self.active = true;

        let (producer, consumer) = rtrb::RingBuffer::new(1024);
        self.param_producer = Some(producer);
        let ctrl_clone = self.controller.clone();

        // A fresh (ref-counted) IAudioProcessor handle for the audio thread.
        let audio_processor = self
            .component
            .cast::<IAudioProcessor>()
            .ok_or_else(|| HostError::Load("plugin has no IAudioProcessor".into()))?;
        Ok(Box::new(Vst3AudioProcessor::new(
            audio_processor,
            self.module.keepalive(),
            &in_channels,
            &out_channels,
            config.max_block_frames,
            consumer,
            ctrl_clone,
        )))
    }

    fn deactivate(&mut self) {
        if self.active {
            unsafe { self.component.setActive(0) };
            self.active = false;
        }
        self.param_producer = None;
    }

    fn param_producer(&mut self) -> Option<&mut rtrb::Producer<crate::host::ParamChange>> {
        self.param_producer.as_mut()
    }

    fn take_param_producer(&mut self) -> Option<rtrb::Producer<crate::host::ParamChange>> {
        self.param_producer.take()
    }

    fn set_param(&mut self, id: u32, value: f64) -> Result<(), HostError> {
        if let Some(controller) = &self.controller {
            let normalized = unsafe { controller.plainParamToNormalized(id, value) };
            unsafe { controller.setParamNormalized(id, normalized) };
        }
        if let Some(producer) = &mut self.param_producer {
            let _ = producer.push(crate::host::ParamChange {
                id,
                value,
                offset_frames: 0,
            });
        }
        Ok(())
    }

    fn params(&mut self) -> Vec<crate::host::ParamInfo> {
        self.controller.as_ref().map(crate::vst3::params::enumerate).unwrap_or_default()
    }

    fn param_value(&mut self, id: u32) -> Option<f64> {
        self.controller.as_ref().and_then(|c| crate::vst3::params::value(c, id))
    }

    fn param_value_to_text(&mut self, id: u32, value: f64) -> Option<String> {
        self.controller.as_ref().and_then(|c| crate::vst3::params::value_to_text(c, id, value))
    }

    fn param_text_to_value(&mut self, id: u32, text: &str) -> Option<f64> {
        self.controller.as_ref().and_then(|c| crate::vst3::params::text_to_value(c, id, text))
    }

    fn latency(&mut self) -> u32 {
        unsafe { self.processor.getLatencySamples() }
    }

    fn open_editor(&mut self) -> Result<(), HostError> {
        #[cfg(target_os = "macos")]
        {
            if self.editor.is_some() {
                return Ok(());
            }
            let controller = self
                .controller
                .as_ref()
                .ok_or_else(|| HostError::Load("plugin has no edit controller".into()))?;
            self.editor = Some(Vst3Editor::open(controller, "Nexus — Plugin")?);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(HostError::Load(
                "GUI editor embedding is not supported on this platform".into(),
            ))
        }
    }

    fn close_editor(&mut self) {
        #[cfg(target_os = "macos")]
        {
            if let Some(editor) = self.editor.take() {
                editor.close();
            }
        }
    }

    fn presets(&mut self) -> Vec<crate::host::PresetInfo> {
        let unit_info: Option<ComPtr<vst3_sdk::Steinberg::Vst::IUnitInfo>> = self
            .controller
            .as_ref()
            .and_then(|c| c.cast::<vst3_sdk::Steinberg::Vst::IUnitInfo>())
            .or_else(|| self.component.cast::<vst3_sdk::Steinberg::Vst::IUnitInfo>());

        let Some(unit_info) = unit_info else {
            return Vec::new();
        };

        let mut out = Vec::new();
        let list_count = unsafe { unit_info.getProgramListCount() };
        for list_idx in 0..list_count {
            let mut list_info: vst3_sdk::Steinberg::Vst::ProgramListInfo = unsafe { std::mem::zeroed() };
            if unsafe { unit_info.getProgramListInfo(list_idx, &mut list_info) } == kResultOk {
                for prog_idx in 0..list_info.programCount {
                    let mut name_buf: vst3_sdk::Steinberg::Vst::String128 = [0; 128];
                    if unsafe {
                        unit_info.getProgramName(list_info.id, prog_idx, &mut name_buf)
                    } == kResultOk
                    {
                        let name = string_from_utf16(&name_buf);
                        out.push(crate::host::PresetInfo {
                            index: prog_idx as u32,
                            name,
                            program_list_id: Some(list_info.id),
                            category: None,
                        });
                    }
                }
            }
        }
        out
    }

    fn load_preset(&mut self, target: &str) -> Result<(), HostError> {
        let presets = self.presets();
        let found = if let Ok(idx) = target.parse::<u32>() {
            presets.iter().find(|p| p.index == idx)
        } else {
            presets
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(target))
                .or_else(|| presets.iter().find(|p| p.name.to_lowercase().contains(&target.to_lowercase())))
        };

        let Some(preset) = found else {
            return Err(HostError::Load(format!(
                "preset {target:?} not found (plugin reports {} presets)",
                presets.len()
            )));
        };

        let controller = self
            .controller
            .as_ref()
            .ok_or_else(|| HostError::Load("plugin has no edit controller".into()))?;

        // Find the program change parameter in controller
        let count = unsafe { controller.getParameterCount() };
        let mut prog_param_id = None;
        for i in 0..count {
            let mut info: vst3_sdk::Steinberg::Vst::ParameterInfo = unsafe { std::mem::zeroed() };
            if unsafe { controller.getParameterInfo(i, &mut info) } == kResultOk {
                if info.flags & vst3_sdk::Steinberg::Vst::ParameterInfo_::ParameterFlags_::kIsProgramChange != 0 {
                    prog_param_id = Some(info.id);
                    break;
                }
            }
        }

        let param_id = prog_param_id.ok_or_else(|| {
            HostError::Load("plugin has no program change parameter".into())
        })?;
        let total = presets
            .iter()
            .filter(|x| x.program_list_id == preset.program_list_id)
            .count();
        let norm = if total > 1 {
            preset.index as f64 / (total - 1) as f64
        } else {
            0.0
        };

        unsafe { controller.setParamNormalized(param_id, norm) };
        Ok(())
    }

    fn pump(&mut self) {
        if let Some(editor) = &self.editor {
            editor.fit();
        }
    }

    fn save_state(&mut self) -> Result<Vec<u8>, HostError> {
        let stream = HostStream::empty();
        let ptr = stream
            .to_com_ptr::<IBStream>()
            .ok_or_else(|| HostError::Preset("could not create state stream".into()))?;
        let result = unsafe { self.component.getState(ptr.as_ptr()) };
        if result != kResultOk {
            return Err(HostError::Preset(format!("getState failed ({result})")));
        }
        Ok(stream.to_vec())
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), HostError> {
        let stream = HostStream::from_bytes(data);
        let ptr = stream
            .to_com_ptr::<IBStream>()
            .ok_or_else(|| HostError::Preset("could not create state stream".into()))?;
        let result = unsafe { self.component.setState(ptr.as_ptr()) };
        if result != kResultOk {
            return Err(HostError::Preset(format!("setState failed ({result})")));
        }
        // Sync the controller's parameters from the (rewound) component state.
        if let Some(controller) = &self.controller {
            let stream = HostStream::from_bytes(data);
            if let Some(ptr) = stream.to_com_ptr::<IBStream>() {
                unsafe { controller.setComponentState(ptr.as_ptr()) };
            }
        }
        Ok(())
    }
}

impl Drop for Vst3PluginInstance {
    fn drop(&mut self) {
        self.teardown();
    }
}

/// Obtain the edit controller (the component itself, or a separate class) and
/// connect it to the component. Best effort: returns `None` if unavailable.
unsafe fn setup_controller(
    module: &Vst3Module,
    component: &ComPtr<IComponent>,
    ctx: *mut FUnknown,
) -> Option<ComPtr<IEditController>> {
    let controller = component.cast::<IEditController>().or_else(|| {
        let mut cid: TUID = [0; 16];
        if unsafe { component.getControllerClassId(&mut cid) } != kResultOk {
            return None;
        }
        let controller = module.create_instance::<IEditController>(&cid)?;
        unsafe { controller.initialize(ctx) };
        Some(controller)
    })?;

    // Wire the component ↔ controller connection points (if both expose them).
    if let (Some(comp_cp), Some(ctrl_cp)) =
        (component.cast::<IConnectionPoint>(), controller.cast::<IConnectionPoint>())
    {
        unsafe {
            comp_cp.connect(ctrl_cp.as_ptr());
            ctrl_cp.connect(comp_cp.as_ptr());
        }
    }

    Some(controller)
}

/// Channel count of audio bus `index` in `dir`, or 0 if unavailable.
unsafe fn bus_channels(component: &ComPtr<IComponent>, dir: BusDirection, index: i32) -> usize {
    let audio = MediaTypes_::kAudio as MediaType;
    let mut info: BusInfo = unsafe { std::mem::zeroed() };
    if unsafe { component.getBusInfo(audio, dir, index, &mut info) } == kResultOk {
        info.channelCount.max(0) as usize
    } else {
        0
    }
}

/// Speaker arrangement for bus `index` in `dir`, from its channel count.
unsafe fn arrangement(
    component: &ComPtr<IComponent>,
    dir: BusDirection,
    index: i32,
) -> SpeakerArrangement {
    let audio = MediaTypes_::kAudio as MediaType;
    let mut info: BusInfo = unsafe { std::mem::zeroed() };
    let channels = if unsafe { component.getBusInfo(audio, dir, index, &mut info) } == kResultOk {
        info.channelCount
    } else {
        2
    };
    match channels {
        0 => SpeakerArr::kEmpty,
        1 => SpeakerArr::kMono,
        _ => SpeakerArr::kStereo,
    }
}

/// Parse a 32-char hex class id (as the probe emits) back into a [`TUID`]
/// (`char8`/`i8` bytes).
fn parse_cid(hex: &str) -> Option<TUID> {
    if hex.len() != 32 {
        return None;
    }
    let mut cid = [0i8; 16];
    for (i, byte) in cid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()? as i8;
    }
    Some(cid)
}

fn string_from_utf16(buf: &[vst3_sdk::Steinberg::Vst::TChar]) -> String {
    let units: Vec<u16> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u16)
        .collect();
    String::from_utf16_lossy(&units)
}
