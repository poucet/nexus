//! The host's `IComponentHandler`, through which a VST3 edit controller tells
//! the host about edits in its editor and asks for restarts.
//!
//! VST3 calls it on the UI thread, which for nexus is the instance's owning
//! thread; a plugin that calls from another thread is still safe, because the
//! handler only queues the call ([`HandlerCall`]) on a channel. The instance's
//! `pump` turns the queue into [`Notice`]s, asking the controller for plain
//! values and the processor for its latency outside the plugin's own call.

use crossbeam_channel::{Receiver, Sender, unbounded};
use vst3_sdk::Steinberg::Vst::{
    IComponentHandler, IComponentHandlerTrait, ParamID, ParamValue, RestartFlags_,
};
use vst3_sdk::Steinberg::{int32, kResultOk, tresult};
use vst3_sdk::{Class, ComWrapper};

use crate::notice::{Notice, Rescan};

/// One call the controller made, as queued for `pump`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HandlerCall {
    /// `beginEdit(id)`.
    Begin(u32),
    /// `performEdit(id, normalized)`.
    Perform(u32, f64),
    /// `endEdit(id)`.
    End(u32),
    /// `restartComponent(flags)`.
    Restart(i32),
}

/// Nexus's `IComponentHandler`.
pub struct Vst3ComponentHandler {
    calls: Sender<HandlerCall>,
}

impl Vst3ComponentHandler {
    /// The handler, as a COM object, and the queue its calls arrive on.
    pub fn new() -> (ComWrapper<Vst3ComponentHandler>, Receiver<HandlerCall>) {
        let (calls, receiver) = unbounded();
        (ComWrapper::new(Vst3ComponentHandler { calls }), receiver)
    }

    fn queue(&self, call: HandlerCall) -> tresult {
        let _ = self.calls.send(call);
        kResultOk
    }
}

impl Class for Vst3ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Vst3ComponentHandler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.queue(HandlerCall::Begin(id))
    }

    unsafe fn performEdit(&self, id: ParamID, value_normalized: ParamValue) -> tresult {
        self.queue(HandlerCall::Perform(id, value_normalized))
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        self.queue(HandlerCall::End(id))
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        self.queue(HandlerCall::Restart(flags))
    }
}

/// The notices `restartComponent(flags)` means, in a fixed order: restart,
/// latency (asked of `latency` only when it changed), then one rescan.
pub fn restart_notices(flags: i32, latency: impl FnOnce() -> u32, out: &mut Vec<Notice>) {
    use RestartFlags_::*;
    let has = |flag: i32| flags & flag != 0;
    if has(kReloadComponent) {
        out.push(Notice::Restart);
    }
    if has(kLatencyChanged) {
        out.push(Notice::Latency(latency()));
    }
    let rescan = Rescan {
        values: has(kParamValuesChanged),
        text: has(kParamTitlesChanged),
        info: has(kParamTitlesChanged),
        list: has(kParamIDMappingChanged),
        io: has(kIoChanged) || has(kIoTitlesChanged),
        midi_mapping: has(kMidiCCAssignmentChanged),
    };
    if rescan.any() {
        out.push(Notice::Rescan(rescan));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3_sdk::Steinberg::Vst::IComponentHandler;

    /// What a controller does, through the COM pointer it is given.
    #[test]
    fn controller_calls_arrive_in_order() {
        let (handler, calls) = Vst3ComponentHandler::new();
        let ptr = handler.to_com_ptr::<IComponentHandler>().unwrap();
        unsafe {
            assert_eq!(ptr.beginEdit(3), kResultOk);
            ptr.performEdit(3, 0.25);
            ptr.performEdit(3, 0.5);
            ptr.endEdit(3);
            ptr.restartComponent(RestartFlags_::kLatencyChanged);
        }
        let got: Vec<_> = calls.try_iter().collect();
        assert_eq!(
            got,
            [
                HandlerCall::Begin(3),
                HandlerCall::Perform(3, 0.25),
                HandlerCall::Perform(3, 0.5),
                HandlerCall::End(3),
                HandlerCall::Restart(RestartFlags_::kLatencyChanged),
            ]
        );
    }

    #[test]
    fn a_latency_change_is_one_notice() {
        let mut out = Vec::new();
        restart_notices(RestartFlags_::kLatencyChanged, || 64, &mut out);
        assert_eq!(out, [Notice::Latency(64)]);
    }

    #[test]
    fn restart_flags_become_a_restart_and_one_rescan() {
        use RestartFlags_::*;
        let mut out = Vec::new();
        let flags = kReloadComponent | kParamValuesChanged | kParamTitlesChanged | kIoChanged;
        restart_notices(flags, || panic!("latency did not change"), &mut out);
        assert_eq!(
            out,
            [
                Notice::Restart,
                Notice::Rescan(Rescan {
                    values: true,
                    text: true,
                    info: true,
                    io: true,
                    ..Default::default()
                }),
            ]
        );
        out.clear();
        restart_notices(kNoteExpressionChanged, || 0, &mut out);
        assert!(out.is_empty(), "nothing a host reads");
    }
}
