//! Host-side implementation of VST3 `IParameterChanges` and `IParamValueQueue`.
//!
//! Provides parameter changes to a VST3 plugin during `IAudioProcessor::process`.

use std::cell::UnsafeCell;

use vst3_sdk::Steinberg::Vst::{
    IParamValueQueue, IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID,
    ParamValue,
};
use vst3_sdk::Steinberg::{int32, kInvalidArgument, kResultOk, tresult};
use vst3_sdk::{Class, ComPtr, ComWrapper};

#[derive(Debug, Clone, Copy)]
pub struct ParamPoint {
    pub sample_offset: i32,
    pub value: f64,
}

pub struct HostParamQueue {
    param_id: UnsafeCell<ParamID>,
    points: UnsafeCell<Vec<ParamPoint>>,
}

unsafe impl Send for HostParamQueue {}
unsafe impl Sync for HostParamQueue {}

impl HostParamQueue {
    pub fn new(param_id: ParamID, capacity: usize) -> ComWrapper<HostParamQueue> {
        ComWrapper::new(HostParamQueue {
            param_id: UnsafeCell::new(param_id),
            points: UnsafeCell::new(Vec::with_capacity(capacity)),
        })
    }

    pub fn set(&self, param_id: ParamID, points: &[ParamPoint]) {
        unsafe {
            *self.param_id.get() = param_id;
            let pts = &mut *self.points.get();
            pts.clear();
            pts.extend_from_slice(points);
        }
    }

    pub fn add(&self, point: ParamPoint) {
        unsafe {
            (*self.points.get()).push(point);
        }
    }

    pub fn clear(&self) {
        unsafe {
            (*self.points.get()).clear();
        }
    }
}

impl Class for HostParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for HostParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        unsafe { *self.param_id.get() }
    }

    unsafe fn getPointCount(&self) -> int32 {
        unsafe { (*self.points.get()).len() as int32 }
    }

    unsafe fn getPoint(&self, index: int32, sample_offset: *mut int32, value: *mut ParamValue) -> tresult {
        let pts = unsafe { &*self.points.get() };
        if let Some(pt) = pts.get(index as usize) {
            if !sample_offset.is_null() {
                unsafe { *sample_offset = pt.sample_offset };
            }
            if !value.is_null() {
                unsafe { *value = pt.value };
            }
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn addPoint(&self, sample_offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let pts = unsafe { &mut *self.points.get() };
        let idx = pts.len() as int32;
        pts.push(ParamPoint { sample_offset, value });
        if !index.is_null() {
            unsafe { *index = idx };
        }
        kResultOk
    }
}

pub struct HostParameterChanges {
    queues: UnsafeCell<Vec<ComWrapper<HostParamQueue>>>,
    ptrs: UnsafeCell<Vec<ComPtr<IParamValueQueue>>>,
}

unsafe impl Send for HostParameterChanges {}
unsafe impl Sync for HostParameterChanges {}

impl HostParameterChanges {
    pub fn new(capacity: usize) -> ComWrapper<HostParameterChanges> {
        let mut queues = Vec::with_capacity(capacity);
        let mut ptrs = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            let q = HostParamQueue::new(0, 16);
            ptrs.push(q.to_com_ptr::<IParamValueQueue>().unwrap());
            queues.push(q);
        }
        ComWrapper::new(HostParameterChanges {
            queues: UnsafeCell::new(queues),
            ptrs: UnsafeCell::new(ptrs),
        })
    }

    pub fn clear(&self) {
        let queues = unsafe { &*self.queues.get() };
        for q in queues {
            q.clear();
        }
    }

    pub fn set_changes(&self, changes: &[(ParamID, Vec<ParamPoint>)]) {
        let queues = unsafe { &mut *self.queues.get() };
        let ptrs = unsafe { &mut *self.ptrs.get() };

        while queues.len() < changes.len() {
            let q = HostParamQueue::new(0, 16);
            ptrs.push(q.to_com_ptr::<IParamValueQueue>().unwrap());
            queues.push(q);
        }

        for (i, (id, pts)) in changes.iter().enumerate() {
            queues[i].set(*id, pts);
        }
        // Clear any unused queues
        for i in changes.len()..queues.len() {
            queues[i].clear();
        }
    }

    pub fn active_count(&self) -> usize {
        let queues = unsafe { &*self.queues.get() };
        queues.iter().take_while(|q| unsafe { q.getPointCount() > 0 }).count()
    }
}

impl Class for HostParameterChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for HostParameterChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        let queues = unsafe { &*self.queues.get() };
        let mut count = 0;
        for q in queues {
            if unsafe { q.getPointCount() > 0 } {
                count += 1;
            }
        }
        count
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        let queues = unsafe { &*self.queues.get() };
        let ptrs = unsafe { &*self.ptrs.get() };
        let mut active_idx = 0;
        for (i, q) in queues.iter().enumerate() {
            if unsafe { q.getPointCount() > 0 } {
                if active_idx == index {
                    return ptrs[i].as_ptr();
                }
                active_idx += 1;
            }
        }
        std::ptr::null_mut()
    }

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        let _ = (id, index);
        std::ptr::null_mut()
    }
}
