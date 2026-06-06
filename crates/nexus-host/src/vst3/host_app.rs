//! The host context object handed to a VST3 plugin at `initialize`.
//!
//! A plugin queries this for the host's name and (for inter-component
//! messaging) to create `IMessage`/`IAttributeList` objects. We provide the
//! name; message creation is not yet implemented (most plugins tolerate this —
//! single-component effects and synths don't need it to make sound).

use std::ffi::c_void;

use vst3_sdk::Steinberg::Vst::{IHostApplication, IHostApplicationTrait, String128, TChar};
use vst3_sdk::Steinberg::{TUID, kNotImplemented, kResultOk, tresult};
use vst3_sdk::{Class, ComWrapper};

/// Nexus' implementation of `IHostApplication`.
pub struct Vst3HostApp;

impl Vst3HostApp {
    /// Build the host context as a COM object.
    pub fn new() -> ComWrapper<Vst3HostApp> {
        ComWrapper::new(Vst3HostApp)
    }
}

impl Class for Vst3HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for Vst3HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kNotImplemented;
        }
        copy_utf16("Nexus", unsafe { &mut *name });
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        _cid: *mut TUID,
        _iid: *mut TUID,
        _obj: *mut *mut c_void,
    ) -> tresult {
        // IMessage / IAttributeList creation — wire up alongside full
        // component↔controller messaging later.
        kNotImplemented
    }
}

/// Copy `src` into a null-terminated UTF-16 (`TChar`) buffer.
pub fn copy_utf16(src: &str, dst: &mut [TChar]) {
    let mut i = 0;
    for unit in src.encode_utf16() {
        if i + 1 >= dst.len() {
            break;
        }
        dst[i] = unit as TChar;
        i += 1;
    }
    if i < dst.len() {
        dst[i] = 0;
    }
}
