//! A host-side `IEventList` the plugin reads input events from each block.
//!
//! Filled on the audio thread just before `process` from the façade's
//! [`EventBuffer`](crate::process::EventBuffer); the plugin then pulls events
//! via `getEventCount` / `getEvent`.

use std::cell::UnsafeCell;

use vst3_sdk::Steinberg::Vst::{Event, IEventList, IEventListTrait};
use vst3_sdk::Steinberg::{int32, kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3_sdk::{Class, ComWrapper};

/// A reusable input event list owned by the audio processor.
pub struct HostEventList {
    events: UnsafeCell<Vec<Event>>,
}

// SAFETY: only ever touched on the single audio thread that owns the processor
// (which is itself `unsafe impl Send`); never shared concurrently.
unsafe impl Send for HostEventList {}
unsafe impl Sync for HostEventList {}

impl HostEventList {
    /// Create the list with room for `capacity` events (no per-block alloc).
    pub fn new(capacity: usize) -> ComWrapper<HostEventList> {
        ComWrapper::new(HostEventList { events: UnsafeCell::new(Vec::with_capacity(capacity)) })
    }

    /// Replace this block's events. Audio-thread only.
    pub fn set(&self, events: &[Event]) {
        // SAFETY: exclusive audio-thread access; the plugin only reads, between
        // `set` calls.
        let buf = unsafe { &mut *self.events.get() };
        buf.clear();
        buf.extend_from_slice(events);
    }
}

impl Class for HostEventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for HostEventList {
    unsafe fn getEventCount(&self) -> int32 {
        unsafe { (*self.events.get()).len() as int32 }
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        let buf = unsafe { &*self.events.get() };
        match buf.get(index as usize) {
            Some(ev) if !e.is_null() => {
                unsafe { *e = *ev };
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn addEvent(&self, _e: *mut Event) -> tresult {
        // The host's input list is read-only to the plugin.
        kResultFalse
    }
}
