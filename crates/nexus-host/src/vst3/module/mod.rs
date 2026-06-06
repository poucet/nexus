//! Loading a VST3 bundle and obtaining its plugin factory.
//!
//! Bundle layout and entry points differ per OS (macOS `CFBundle`/`bundleEntry`,
//! Windows `LoadLibrary`/`InitDll`, Linux `dlopen`/`ModuleEntry`), so — like
//! [`crate::editor_window`] — **this is the only place in the VST3 backend that
//! branches on `target_os`**. It exposes two things, both backed by a per-OS
//! [`LoadedModule`] keep-alive so the factory never outlives its binary:
//!
//! - [`Vst3Factory`] — load just far enough to read the factory (cataloguing).
//! - [`Vst3Module`] — load + initialize for hosting, with a shareable handle.

use std::path::Path;
use std::sync::Arc;

use vst3_sdk::ComPtr;
use vst3_sdk::Steinberg::IPluginFactory;

use crate::host::HostError;

/// Keeps a loaded VST3 module's code (and its resource bundle) alive. Opaque and
/// `Send + Sync` so the audio processor can pin it for as long as it holds COM
/// objects from the module — a released COM object must never call into an
/// unloaded module.
pub trait LoadedModule: Send + Sync {}

/// A plugin factory plus the module backing it, loaded **only to read the
/// factory** (e.g. to catalogue a bundle's classes). Runs no module init and
/// drops the module when it goes out of scope. To actually host a plugin, use
/// [`Vst3Module`].
pub struct Vst3Factory {
    /// Declared before `_module` so it is released before the module unloads.
    factory: ComPtr<IPluginFactory>,
    _module: Box<dyn LoadedModule>,
}

impl Vst3Factory {
    /// Load the bundle and read its factory (no module init).
    pub fn load(path: &Path) -> Result<Self, HostError> {
        let (module, factory) = platform::probe(path)?;
        Ok(Self { factory, _module: module })
    }

    /// The plugin factory, valid while this `Vst3Factory` is alive.
    pub fn factory(&self) -> &ComPtr<IPluginFactory> {
        &self.factory
    }
}

/// A loaded, initialized VST3 module for **hosting**: its factory plus a
/// keep-alive handle pinning the binary for the plugin's life.
pub struct Vst3Module {
    /// Released before `handle` (declared after) drops, so it always calls into
    /// a still-loaded module.
    pub factory: ComPtr<IPluginFactory>,
    handle: Arc<dyn LoadedModule>,
}

impl Vst3Module {
    /// Load and initialize for hosting — runs the module's init entry with a
    /// real resource bundle so the plugin can find its resources.
    pub fn host(path: &Path) -> Result<Self, HostError> {
        let (handle, factory) = platform::host(path)?;
        Ok(Self { factory, handle })
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
     Contents/x86_64-win DLL, call InitDll, then GetPluginFactory."
);

#[cfg(target_os = "linux")]
compile_error!(
    "vst3::module: Linux loading not implemented yet — dlopen the \
     Contents/x86_64-linux .so, call ModuleEntry, then GetPluginFactory."
);

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("vst3::module: no loader for this target OS.");
