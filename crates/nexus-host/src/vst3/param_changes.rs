//! Host-side implementation of VST3 `IParameterChanges` and `IParamValueQueue`.
//!
//! Provides parameter changes to a VST3 plugin during `IAudioProcessor::process`.
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
}

// SAFETY: as for `HostParamQueue`.
unsafe impl Send for HostParameterChanges {}
unsafe impl Sync for HostParameterChanges {}

impl HostParameterChanges {
    /// Allocate [`QUEUES`] queues of [`POINTS`] points each. Not realtime.
    pub fn new() -> ComWrapper<HostParameterChanges> {
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
        let active = self.active.get();
        let point = ParamPoint { sample_offset, value };
        let kept = match self.queues[..active].iter().find(|q| q.param_id.get() == id) {
            Some(queue) => queue.insert(point),
            None if active < self.queues.len() => {
                let queue = &self.queues[active];
                queue.param_id.set(id);
                self.active.set(active + 1);
                queue.insert(point)
            }
            None => false,
        };
        if !kept {
            self.dropped.set(self.dropped.get() + 1);
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

    unsafe fn addParameterData(&self, _id: *const ParamID, _index: *mut int32) -> *mut IParamValueQueue {
        // The host's input changes are read-only to the plugin.
        std::ptr::null_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
