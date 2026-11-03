//! The host's `IComponentHandler`, through which a VST3 edit controller tells
//! the host about edits in its editor and asks for restarts.
//!
//! VST3 calls it on the UI thread, which for nexus is the instance's owning
//! thread; a plugin that calls from another thread is still safe, because the
//! handler only queues the call ([`HandlerCall`]) on a channel. The instance's
//! `pump` hands the queue to [`HandlerQueue::drain`], which turns it into
//! [`Notice`]s, asking the controller for plain values and the processor for
//! its latency outside the plugin's own call, and forwards each edit to the
//! processor: in VST3 the processor hears of an editor edit only from the host.

use crossbeam_channel::{Receiver, Sender, unbounded};
use vst3_sdk::Steinberg::Vst::{
    IComponentHandler, IComponentHandlerTrait, ParamID, ParamValue, RestartFlags_,
};
use vst3_sdk::Steinberg::{int32, kResultOk, tresult};
use vst3_sdk::{Class, ComWrapper};

use crate::notice::{Notice, NoticeSink, Rescan};
use crate::vst3::param_changes::PendingParams;
use crate::vst3::process::ControllerEdit;

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
    /// `IPlugFrame::resizeView` from the editor, already accepted.
    ResizeView(crate::EditorSize),
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

    /// Where the editor's `IPlugFrame` queues its calls: the same queue.
    pub fn calls(&self) -> Sender<HandlerCall> {
        self.calls.clone()
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

/// The owning thread's end of the handler: its calls, and the ring that
/// forwards edits to the processor while the plugin is active.
pub struct HandlerQueue {
    calls: Receiver<HandlerCall>,
    edits: Option<rtrb::Producer<ControllerEdit>>,
}

impl HandlerQueue {
    pub fn new(calls: Receiver<HandlerCall>) -> Self {
        Self { calls, edits: None }
    }

    /// Activation: edits go to the processor through a ring of `capacity`,
    /// whose other end is returned for it.
    pub fn activate(&mut self, capacity: usize) -> rtrb::Consumer<ControllerEdit> {
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        self.edits = Some(producer);
        consumer
    }

    /// Deactivation: edits wait in the pending values for the next activation.
    pub fn deactivate(&mut self) {
        self.edits = None;
    }

    /// Turn the calls queued since the last drain into notices in `out`, and
    /// forward each edit: to the processor while active, to `pending` (the
    /// first block after activation) otherwise. `plain` is the controller's
    /// normalised → plain; `latency` the processor's latency. Returns the
    /// restart flags seen, OR-ed.
    pub fn drain(
        &mut self,
        plain: impl Fn(u32, f64) -> f64,
        latency: impl Fn() -> u32,
        pending: &mut PendingParams,
        out: &mut impl NoticeSink,
    ) -> i32 {
        let mut restarts = 0;
        while let Ok(call) = self.calls.try_recv() {
            match call {
                HandlerCall::Begin(id) => _ = out.notice(Notice::GestureBegin(id)),
                HandlerCall::End(id) => _ = out.notice(Notice::GestureEnd(id)),
                HandlerCall::Perform(id, normalized) => {
                    let value = plain(id, normalized);
                    out.notice(Notice::ParamValue { id, value });
                    match self.edits.as_mut() {
                        None => pending.set(id, value),
                        Some(ring) => {
                            if ring.push(ControllerEdit { id, normalized }).is_err() {
                                log::warn!("VST3 editor edit of parameter {id} dropped: the edit ring is full");
                            }
                        }
                    }
                }
                HandlerCall::Restart(flags) => {
                    restart_notices(flags, &latency, out);
                    restarts |= flags;
                }
                HandlerCall::ResizeView(size) => _ = out.notice(Notice::EditorResize(size)),
            }
        }
        restarts
    }
}

/// The notices `restartComponent(flags)` means, in a fixed order: restart,
/// latency (asked of `latency` only when it changed), then one rescan.
pub fn restart_notices(flags: i32, latency: impl Fn() -> u32, out: &mut impl NoticeSink) {
    use RestartFlags_::*;
    let has = |flag: i32| flags & flag != 0;
    if has(kReloadComponent) {
        out.notice(Notice::Restart);
    }
    if has(kLatencyChanged) {
        out.notice(Notice::Latency(latency()));
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
        out.notice(Notice::Rescan(rescan));
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

    /// An editor drag, as a controller makes it: the host hears a gesture
    /// with plain values, and each value is forwarded to the processor while
    /// active, or kept for the first block while not.
    #[test]
    fn an_editor_drag_is_a_gesture_and_reaches_the_processor() {
        let (handler, calls) = Vst3ComponentHandler::new();
        let ptr = handler.to_com_ptr::<IComponentHandler>().unwrap();
        let drag = |id: u32, values: &[f64]| unsafe {
            ptr.beginEdit(id);
            for &v in values {
                ptr.performEdit(id, v);
            }
            ptr.endEdit(id);
        };
        let mut queue = HandlerQueue::new(calls);
        let mut pending = PendingParams::default();
        let plain = |_: u32, x: f64| 20.0 + x * 19_980.0;
        let mut out = Vec::new();

        let mut processor = queue.activate(16);
        drag(4, &[0.0, 0.5]);
        queue.drain(plain, || 0, &mut pending, &mut out);
        assert_eq!(
            out,
            [
                Notice::GestureBegin(4),
                Notice::ParamValue { id: 4, value: 20.0 },
                Notice::ParamValue { id: 4, value: 10_010.0 },
                Notice::GestureEnd(4),
            ]
        );
        let forwarded: Vec<_> = std::iter::from_fn(|| processor.pop().ok()).collect();
        assert_eq!(
            forwarded,
            [ControllerEdit { id: 4, normalized: 0.0 }, ControllerEdit { id: 4, normalized: 0.5 }]
        );
        assert!(pending.is_empty());

        queue.deactivate();
        out.clear();
        drag(4, &[1.0]);
        unsafe { ptr.restartComponent(RestartFlags_::kLatencyChanged) };
        let restarts = queue.drain(plain, || 32, &mut pending, &mut out);
        assert_eq!(restarts, RestartFlags_::kLatencyChanged);
        assert_eq!(out[1], Notice::ParamValue { id: 4, value: 20_000.0 });
        assert_eq!(out[3], Notice::Latency(32));
        let (_, mut first_block) = pending.ring(4);
        assert_eq!(first_block.pop().map(|c| (c.id, c.value)), Ok((4, 20_000.0)));
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
