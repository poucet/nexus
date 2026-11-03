//! Host-side implementation of VST3 `IParameterChanges` and `IParamValueQueue`.
//!
//! Provides parameter changes to a VST3 plugin during `IAudioProcessor::process`
//! ([`HostParameterChanges::new`]), and takes the ones it writes back
//! ([`HostParameterChanges::output`], `outputParameterChanges`).
//! Every queue and every point slot is allocated when the processor is built;
//! a block only [`begin`](HostParameterChanges::begin)s and
//! [`push`](HostParameterChanges::push)es, which never allocates (`I-100`).
//! What does not fit — more distinct parameters in one block than there are
//! queues, or more points for one parameter than a queue holds — is dropped and
//! counted, keeping the latest value of each parameter.

use std::cell::{Cell, UnsafeCell};

use vst3_sdk::Steinberg::Vst::{
    IParamValueQueue, IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID,
    ParamValue,
};
use vst3_sdk::Steinberg::{int32, kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3_sdk::{Class, ComPtr, ComWrapper};

use crate::notice::{Notice, NoticeSink};
use crate::vst3::params::Normalizer;

/// Distinct parameters one block can carry.
pub const QUEUES: usize = 256;
/// Points one parameter can carry in one block.
pub const POINTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamPoint {
    pub sample_offset: i32,
    pub value: f64,
}

/// One parameter's points for one block, in sample-offset order.
pub struct HostParamQueue {
    param_id: Cell<ParamID>,
    /// Capacity [`POINTS`], never grown.
    points: UnsafeCell<Vec<ParamPoint>>,
}

// SAFETY: owned by the single audio thread that owns the processor (itself
// `unsafe impl Send`); the plugin reads it only inside that thread's `process`.
unsafe impl Send for HostParamQueue {}
unsafe impl Sync for HostParamQueue {}

impl HostParamQueue {
    fn new() -> ComWrapper<HostParamQueue> {
        ComWrapper::new(HostParamQueue {
            param_id: Cell::new(0),
            points: UnsafeCell::new(Vec::with_capacity(POINTS)),
        })
    }

    #[allow(clippy::mut_from_ref)] // the single-thread contract above
    fn points(&self) -> &mut Vec<ParamPoint> {
        // SAFETY: see the `Send`/`Sync` note; no reference outlives a call.
        unsafe { &mut *self.points.get() }
    }

    /// Insert `point` in offset order (after points at the same offset). When
    /// full, a point at or after the last one replaces it, so the block still
    /// ends on the newest value; an earlier one is dropped. Returns whether it
    /// was kept.
    fn insert(&self, point: ParamPoint) -> bool {
        let points = self.points();
        let at = points.partition_point(|p| p.sample_offset <= point.sample_offset);
        if points.len() < points.capacity() {
            points.insert(at, point); // within capacity: no allocation
            true
        } else if at == points.len() {
            *points.last_mut().expect("capacity > 0") = point;
            true
        } else {
            false
        }
    }
}

impl Class for HostParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for HostParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.param_id.get()
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points().len() as int32
    }

    unsafe fn getPoint(&self, index: int32, sample_offset: *mut int32, value: *mut ParamValue) -> tresult {
        let Some(pt) = self.points().get(index as usize).copied() else {
            return kInvalidArgument;
        };
        if !sample_offset.is_null() {
            unsafe { *sample_offset = pt.sample_offset };
        }
        if !value.is_null() {
            unsafe { *value = pt.value };
        }
        kResultOk
    }

    unsafe fn addPoint(&self, sample_offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        if !self.insert(ParamPoint { sample_offset, value }) {
            return kResultFalse;
        }
        if !index.is_null() {
            let at = self.points().iter().rposition(|p| p.sample_offset == sample_offset);
            unsafe { *index = at.unwrap_or(0) as int32 };
        }
        kResultOk
    }
}

/// The block's parameter changes: the first [`active`](Self::active_count)
/// queues, one per parameter.
pub struct HostParameterChanges {
    queues: Vec<ComWrapper<HostParamQueue>>,
    ptrs: Vec<ComPtr<IParamValueQueue>>,
    active: Cell<usize>,
    dropped: Cell<u64>,
    /// The plugin may add parameters (`outputParameterChanges`).
    writable: bool,
}

// SAFETY: as for `HostParamQueue`.
unsafe impl Send for HostParameterChanges {}
unsafe impl Sync for HostParameterChanges {}

impl HostParameterChanges {
    /// Allocate [`QUEUES`] queues of [`POINTS`] points each, for the host's
    /// changes (read-only to the plugin). Not realtime.
    pub fn new() -> ComWrapper<HostParameterChanges> {
        Self::with_access(false)
    }

    /// The same, for the changes the plugin writes (`outputParameterChanges`):
    /// `addParameterData` hands out the preallocated queues. Not realtime.
    pub fn output() -> ComWrapper<HostParameterChanges> {
        Self::with_access(true)
    }

    fn with_access(writable: bool) -> ComWrapper<HostParameterChanges> {
        let queues: Vec<_> = (0..QUEUES).map(|_| HostParamQueue::new()).collect();
        let ptrs = queues
            .iter()
            .map(|q| q.to_com_ptr::<IParamValueQueue>().expect("a queue is an IParamValueQueue"))
            .collect();
        ComWrapper::new(HostParameterChanges {
            queues,
            ptrs,
            active: Cell::new(0),
            dropped: Cell::new(0),
            writable,
        })
    }

    /// Start a block: no parameter has changed yet.
    pub fn begin(&self) {
        for q in &self.queues[..self.active.get()] {
            q.points().clear();
        }
        self.active.set(0);
    }

    /// Add a normalised `value` for parameter `id` at `sample_offset`.
    pub fn push(&self, id: ParamID, sample_offset: i32, value: f64) {
        let point = ParamPoint { sample_offset, value };
        let kept = self.queue_for(id).is_some_and(|i| self.queues[i].insert(point));
        if !kept {
            self.dropped.set(self.dropped.get() + 1);
        }
    }

    /// The index of parameter `id`'s queue in this block, claiming a free one
    /// if it has none; `None` when every queue is taken.
    fn queue_for(&self, id: ParamID) -> Option<usize> {
        let active = self.active.get();
        if let Some(i) = self.queues[..active].iter().position(|q| q.param_id.get() == id) {
            return Some(i);
        }
        let queue = self.queues.get(active)?;
        queue.param_id.set(id);
        self.active.set(active + 1);
        Some(active)
    }

    /// Every point of this block, in sample-offset order across parameters
    /// (points at one offset in queue order), as plain-value
    /// [`Notice::ParamValue`]s into `sink`. Realtime-safe: no allocation.
    ///
    /// [`Notice::ParamValue`]: crate::Notice::ParamValue
    pub fn drain_notices(&self, normalizer: &Normalizer, sink: &mut impl NoticeSink) {
        let active = self.active.get();
        let mut cursors = [0u16; QUEUES];
        loop {
            let mut next: Option<(usize, i32)> = None;
            for (i, queue) in self.queues[..active].iter().enumerate() {
                let Some(point) = queue.points().get(cursors[i] as usize) else { continue };
                if next.is_none_or(|(_, offset)| point.sample_offset < offset) {
                    next = Some((i, point.sample_offset));
                }
            }
            let Some((i, _)) = next else { return };
            let queue = &self.queues[i];
            let point = queue.points()[cursors[i] as usize];
            cursors[i] += 1;
            let id = queue.param_id.get();
            sink.notice(Notice::ParamValue { id, value: normalizer.plain(id, point.value) });
        }
    }

    /// Parameters changed in this block.
    pub fn active_count(&self) -> usize {
        self.active.get()
    }

    /// Points dropped since the processor was built (see the module docs).
    pub fn dropped(&self) -> u64 {
        self.dropped.get()
    }

    /// Parameter `id`'s points in this block (tests and diagnostics).
    pub fn points(&self, id: ParamID) -> Option<&[ParamPoint]> {
        self.queues[..self.active.get()]
            .iter()
            .find(|q| q.param_id.get() == id)
            .map(|q| q.points().as_slice())
    }
}

impl Class for HostParameterChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for HostParameterChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.active.get() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        match usize::try_from(index) {
            Ok(i) if i < self.active.get() => self.ptrs[i].as_ptr(),
            _ => std::ptr::null_mut(),
        }
    }

    /// Input changes are read-only to the plugin; output changes hand out a
    /// preallocated queue (null once every queue is taken).
    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        if !self.writable || id.is_null() {
            return std::ptr::null_mut();
        }
        let Some(i) = self.queue_for(unsafe { *id }) else {
            self.dropped.set(self.dropped.get() + 1);
            return std::ptr::null_mut();
        };
        if !index.is_null() {
            unsafe { *index = i as int32 };
        }
        self.ptrs[i].as_ptr()
    }
}

/// Parameter values set while the plugin is inactive (`I-105`).
///
/// A VST3 processor hears of a parameter only through a block's
/// `IParameterChanges`, and the ring that feeds those is made at activation.
/// Until then [`set`](Self::set) keeps the newest value per parameter, and
/// [`ring`](Self::ring) starts the activation's ring with them, so the first
/// block plays them. The edit controller is told at once, as before.
#[derive(Debug, Default)]
pub struct PendingParams {
    /// One entry per parameter, in the order each was first set.
    changes: Vec<crate::host::ParamChange>,
}

impl PendingParams {
    /// Keep `value` for `id`, replacing any earlier pending value.
    pub fn set(&mut self, id: u32, value: f64) {
        match self.changes.iter_mut().find(|c| c.id == id) {
            Some(change) => change.value = value,
            None => self.changes.push(crate::host::ParamChange { id, value, offset_frames: 0 }),
        }
    }

    /// Forget every pending value (a loaded state supersedes them).
    pub fn clear(&mut self) {
        self.changes.clear();
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The ring for an activation: room for `capacity` changes (more if more
    /// are pending), holding every pending value at frame 0. Empties `self`.
    pub fn ring(
        &mut self,
        capacity: usize,
    ) -> (rtrb::Producer<crate::host::ParamChange>, rtrb::Consumer<crate::host::ParamChange>) {
        let (mut producer, consumer) = rtrb::RingBuffer::new(capacity.max(self.changes.len()));
        for change in self.changes.drain(..) {
            producer.push(change).expect("the ring was sized for every pending change");
        }
        (producer, consumer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3_sdk::ComRef;

    fn offsets(changes: &HostParameterChanges, id: ParamID) -> Vec<(i32, f64)> {
        changes.points(id).unwrap().iter().map(|p| (p.sample_offset, p.value)).collect()
    }

    #[test]
    fn points_group_by_parameter_in_offset_order() {
        let changes = HostParameterChanges::new();
        changes.begin();
        changes.push(7, 10, 0.1);
        changes.push(3, 0, 0.3);
        changes.push(7, 0, 0.0);
        changes.push(7, 10, 0.2); // same offset: after the earlier one
        assert_eq!(changes.active_count(), 2);
        assert_eq!(offsets(&changes, 7), [(0, 0.0), (10, 0.1), (10, 0.2)]);
        assert_eq!(unsafe { changes.getParameterCount() }, 2);
        assert!(!unsafe { changes.getParameterData(0) }.is_null());
        assert!(unsafe { changes.getParameterData(2) }.is_null());

        changes.begin();
        assert_eq!(changes.active_count(), 0);
        assert!(changes.points(7).is_none());
        assert!(unsafe { changes.getParameterData(0) }.is_null());
    }

    #[test]
    fn the_input_list_is_read_only_to_the_plugin() {
        let changes = HostParameterChanges::new();
        changes.begin();
        let mut index = -1;
        assert!(unsafe { changes.addParameterData(&5, &mut index) }.is_null());
        assert_eq!(changes.active_count(), 0);
    }

    /// What a plugin does with `outputParameterChanges`, through the COM
    /// pointers it is handed; the host reads it back in offset order.
    #[test]
    fn output_changes_come_back_as_plain_notices_in_offset_order() {
        let changes = HostParameterChanges::output();
        changes.begin();
        let ptr = changes.to_com_ptr::<IParameterChanges>().unwrap();
        let mut index = -1;
        let gain = unsafe { ptr.addParameterData(&7, &mut index) };
        assert_eq!(index, 0);
        let mut at = 0;
        unsafe {
            let gain = ComRef::from_raw(gain).unwrap();
            gain.addPoint(0, 0.1, &mut at);
            gain.addPoint(20, 0.3, &mut at);
        }
        let freq = unsafe { ptr.addParameterData(&9, &mut index) };
        assert_eq!(index, 1);
        unsafe { ComRef::from_raw(freq).unwrap().addPoint(10, 0.5, &mut at) };
        assert_eq!(unsafe { ptr.addParameterData(&7, &mut index) }, gain, "the same queue");

        // Parameter 9 is 20..20 000 Hz (linear here); 7 is unlisted (normalised).
        let normalizer = Normalizer::sample([(9, 0)], |_, x| 20.0 + x * 19_980.0);
        let mut out = Vec::new();
        changes.drain_notices(&normalizer, &mut out);
        assert_eq!(
            out,
            [
                Notice::ParamValue { id: 7, value: 0.1 },
                Notice::ParamValue { id: 9, value: 10_010.0 },
                Notice::ParamValue { id: 7, value: 0.3 },
            ]
        );
    }

    #[test]
    fn a_full_queue_keeps_the_newest_value_and_counts_the_rest() {
        let changes = HostParameterChanges::new();
        changes.begin();
        for i in 0..POINTS as i32 + 5 {
            changes.push(1, i, i as f64);
        }
        let points = changes.points(1).unwrap();
        assert_eq!(points.len(), POINTS);
        assert_eq!(points.last().unwrap().value, (POINTS + 4) as f64, "ends on the newest");
        changes.push(1, 0, -1.0); // earlier than the last, and no room
        assert_eq!(changes.dropped(), 1);

        for id in 0..QUEUES as u32 + 3 {
            changes.push(1000 + id, 0, 0.5);
        }
        assert_eq!(changes.active_count(), QUEUES);
        assert_eq!(changes.dropped(), 1 + 4);
    }

    #[test]
    fn pending_values_start_the_activation_ring_newest_first_set_order() {
        let mut pending = PendingParams::default();
        pending.set(7, 0.1);
        pending.set(3, 0.2);
        pending.set(7, 0.9); // replaces 0.1, keeps 7's place
        assert_eq!(pending.len(), 2);

        let (_producer, mut consumer) = pending.ring(1024);
        assert!(pending.is_empty(), "handed over");
        let got: Vec<(u32, f64, u32)> = std::iter::from_fn(|| consumer.pop().ok())
            .map(|c| (c.id, c.value, c.offset_frames))
            .collect();
        assert_eq!(got, [(7, 0.9, 0), (3, 0.2, 0)]);
    }

    #[test]
    fn the_ring_grows_to_hold_every_pending_value() {
        let mut pending = PendingParams::default();
        for id in 0..10 {
            pending.set(id, id as f64);
        }
        let (mut producer, consumer) = pending.ring(4);
        assert_eq!(consumer.slots(), 10);
        assert!(producer.push(crate::host::ParamChange { id: 0, value: 0.0, offset_frames: 0 }).is_err());
    }

    #[test]
    fn a_cleared_set_starts_an_empty_ring() {
        let mut pending = PendingParams::default();
        pending.set(1, 0.5);
        pending.clear();
        let (producer, consumer) = pending.ring(8);
        assert_eq!((producer.slots(), consumer.slots()), (8, 0));
    }
}
