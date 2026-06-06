//! Loading a VST3 bundle and obtaining its plugin factory.
//!
//! Bundle layout and entry points differ per OS (macOS `CFBundle`/`bundleEntry`,
//! Windows `LoadLibrary`/`InitDll`, Linux `dlopen`/`ModuleEntry`), so — like
//! [`crate::editor_window`] — **this is the only place in the VST3 backend that
//! branches on `target_os`**. The flow is:
//!
//! - [`Vst3Factory::load`] loads the binary and reads its factory **without**
//!   running module init — cheap, for cataloguing a bundle's classes.
//! - [`Vst3Factory::host`] promotes that same loaded module into a [`Vst3Module`]
//!   by running its init entry (`bundleEntry`), then you
//!   [`create_instance`](Vst3Module::create_instance) from it. No reload.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;

use vst3_sdk::Steinberg::{FIDString, IPluginFactory, IPluginFactoryTrait, TUID, kResultOk};
use vst3_sdk::{ComPtr, Interface};

use crate::host::HostError;

/// Keeps a loaded VST3 module's code (and its resource bundle) alive, and runs
/// its one-time init entry on demand. Opaque and `Send + Sync` so the audio
/// processor can pin it for as long as it holds COM objects from the module — a
/// released COM object must never call into an unloaded module.
pub trait LoadedModule: Send + Sync {
    /// Run the module's init entry (`bundleEntry` etc.) — called once when a
    /// [`Vst3Factory`] is promoted to a [`Vst3Module`] for hosting.
    fn init(&self);
}

/// A loaded VST3 module and its factory, **without** module init — for reading
/// the factory's classes (cataloguing). Promote it with [`host`](Self::host) to
/// actually host a plugin.
pub struct Vst3Factory {
    /// Declared before `module` so it is released before the module unloads.
    factory: ComPtr<IPluginFactory>,
    module: Box<dyn LoadedModule>,
}

impl Vst3Factory {
    /// Load the bundle and read its factory (no module init).
    pub fn load(path: &Path) -> Result<Self, HostError> {
        let (module, factory) = platform::load(path)?;
        Ok(Self { factory, module })
    }

    /// The plugin factory, valid while this `Vst3Factory` is alive.
    pub fn factory(&self) -> &ComPtr<IPluginFactory> {
        &self.factory
    }

    /// Promote this loaded module to a hosting [`Vst3Module`], running its init
    /// entry (`bundleEntry`) so the plugin is ready to instantiate. Reuses the
    /// already-loaded binary — no reload.
    pub fn host(self) -> Vst3Module {
        self.module.init();
        Vst3Module { factory: self.factory, handle: Arc::from(self.module) }
    }
}

/// A loaded, initialized VST3 module for **hosting**: its factory plus a
/// keep-alive handle pinning the binary for the plugin's life.
pub struct Vst3Module {
    /// Released before `handle` (declared after) drops, so it always calls into
    /// a still-loaded module.
    factory: ComPtr<IPluginFactory>,
    handle: Arc<dyn LoadedModule>,
}

impl Vst3Module {
    /// Create an instance of class `cid`, queried for interface `I`
    /// (e.g. `IComponent` or `IEditController`).
    pub fn create_instance<I: Interface>(&self, cid: &TUID) -> Option<ComPtr<I>> {
        let iid = I::IID;
        let mut obj: *mut c_void = std::ptr::null_mut();
        // SAFETY: FFI; `cid`/`iid` are 16-byte UIDs and `obj` receives the new
        // instance, already queried for interface `I`.
        let result = unsafe {
            self.factory.createInstance(cid.as_ptr(), iid.as_ptr() as FIDString, &mut obj)
        };
        if result == kResultOk && !obj.is_null() {
            unsafe { ComPtr::from_raw(obj as *mut I) }
        } else {
            None
        }
    }

    /// A clone of the keep-alive handle, for anything (e.g. the audio processor)
    /// that outlives this `Vst3Module` but still holds COM objects from it.
    pub fn keepalive(&self) -> Arc<dyn LoadedModule> {
        self.handle.clone()
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(target_os = "windows")]
compile_error!(
    "vst3::module: Windows loading not implemented yet — LoadLibrary the \
     Contents/x86_64-win DLL, run InitDll in LoadedModule::init, GetPluginFactory."
);

#[cfg(target_os = "linux")]
compile_error!(
    "vst3::module: Linux loading not implemented yet — dlopen the \
     Contents/x86_64-linux .so, run ModuleEntry in LoadedModule::init, GetPluginFactory."
);

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("vst3::module: no loader for this target OS.");
