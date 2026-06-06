//! A host-side in-memory `IBStream` for plugin state get/set.
//!
//! `IComponent::getState` writes into one of these; `setState` /
//! `setComponentState` read from one. Used only on the main thread during
//! save/load, so plain `RefCell`/`Cell` interior mutability suffices.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr;

use vst3_sdk::Steinberg::{IBStream, IBStreamTrait, int32, int64, kResultOk, tresult};
use vst3_sdk::{Class, ComWrapper};

/// IBStream `seek` modes (`IStreamSeekMode`).
const SEEK_SET: i32 = 0;
const SEEK_CUR: i32 = 1;
const SEEK_END: i32 = 2;

/// A growable byte buffer exposed to the plugin as an `IBStream`.
pub struct HostStream {
    data: RefCell<Vec<u8>>,
    pos: Cell<usize>,
}

impl HostStream {
    /// An empty stream the plugin can write its state into.
    pub fn empty() -> ComWrapper<HostStream> {
        ComWrapper::new(HostStream { data: RefCell::new(Vec::new()), pos: Cell::new(0) })
    }

    /// A stream pre-filled with `bytes` for the plugin to read its state from.
    pub fn from_bytes(bytes: &[u8]) -> ComWrapper<HostStream> {
        ComWrapper::new(HostStream { data: RefCell::new(bytes.to_vec()), pos: Cell::new(0) })
    }

    /// Snapshot the bytes written so far.
    pub fn to_vec(&self) -> Vec<u8> {
        self.data.borrow().clone()
    }
}

impl Class for HostStream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for HostStream {
    unsafe fn read(
        &self,
        buffer: *mut c_void,
        num_bytes: int32,
        num_bytes_read: *mut int32,
    ) -> tresult {
        let data = self.data.borrow();
        let pos = self.pos.get();
        let n = (num_bytes.max(0) as usize).min(data.len().saturating_sub(pos));
        if n > 0 {
            unsafe { ptr::copy_nonoverlapping(data[pos..].as_ptr(), buffer as *mut u8, n) };
        }
        self.pos.set(pos + n);
        if !num_bytes_read.is_null() {
            unsafe { *num_bytes_read = n as int32 };
        }
        kResultOk
    }

    unsafe fn write(
        &self,
        buffer: *mut c_void,
        num_bytes: int32,
        num_bytes_written: *mut int32,
    ) -> tresult {
        let n = num_bytes.max(0) as usize;
        let pos = self.pos.get();
        {
            let mut data = self.data.borrow_mut();
            if pos + n > data.len() {
                data.resize(pos + n, 0);
            }
            if n > 0 {
                unsafe { ptr::copy_nonoverlapping(buffer as *const u8, data[pos..].as_mut_ptr(), n) };
            }
        }
        self.pos.set(pos + n);
        if !num_bytes_written.is_null() {
            unsafe { *num_bytes_written = n as int32 };
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let len = self.data.borrow().len() as int64;
        let base = match mode {
            SEEK_SET => 0,
            SEEK_CUR => self.pos.get() as int64,
            SEEK_END => len,
            _ => 0,
        };
        let new_pos = (base + pos).max(0);
        self.pos.set(new_pos as usize);
        if !result.is_null() {
            unsafe { *result = new_pos };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if !pos.is_null() {
            unsafe { *pos = self.pos.get() as int64 };
        }
        kResultOk
    }
}
