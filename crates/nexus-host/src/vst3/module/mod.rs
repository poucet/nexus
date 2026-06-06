//! Loading a VST3 bundle and obtaining its plugin factory.
//!
//! Bundle layout and entry points differ per OS (macOS `CFBundle`/`bundleEntry`,
//! Windows `LoadLibrary`/`InitDll`, Linux `dlopen`/`ModuleEntry`), so — like
//! [`crate::editor_window`] — **this is the only place in the VST3 backend that
//! branches on `target_os`** (in the per-OS [`binary_path`](platform) and the
//! host init). It exposes two things:
//!
//! - [`Vst3Factory`] — load just far enough to read the factory (cataloguing).
//! - [`Vst3Module`] — load + initialize for hosting, with a keep-alive handle.

use std::path::Path;
use std::sync::Arc;

use libloading::{Library, Symbol};
use vst3_sdk::ComPtr;
use vst3_sdk::Steinberg::IPluginFactory;

use crate::host::HostError;

/// A plugin factory plus the binary backing it, loaded **only to read the
/// factory** (e.g. to catalogue a bundle's classes). Carries no host-lifetime
/// keep-alive and runs no module init — drop it when done. To actually host a
/// plugin, use [`Vst3Module`].
pub struct Vst3Factory {
    /// Declared before `_library` so it is released before the binary unloads.
    factory: ComPtr<IPluginFactory>,
    _library: Library,
}

impl Vst3Factory {
    /// Load the bundle's binary and read its factory (no module init).
    pub fn load(path: &Path) -> Result<Self, HostError> {
        let library = open(path)?;
        let factory = read_factory(&library)?;
        Ok(Self { factory, _library: library })
    }

    /// The plugin factory, valid while this `Vst3Factory` is alive.
    pub fn factory(&self) -> &ComPtr<IPluginFactory> {
        &self.factory
    }
}

/// Keeps a hosted VST3 module's code (and its resource bundle) alive. Opaque and
/// `Send + Sync` so the audio processor can pin it for as long as it holds COM
/// objects from the module — a released COM object must never call into an
/// unloaded module.
pub trait LoadedModule: Send + Sync {}

/// A loaded, initialized VST3 module for **hosting**: its factory plus a
/// keep-alive handle pinning the binary for the plugin's life.
pub struct Vst3Module {
    /// Released before `handle` (declared after) drops, so it always calls into
    /// a still-loaded module.
    pub factory: ComPtr<IPluginFactory>,
    handle: Arc<dyn LoadedModule>,
}

impl Vst3Module {
    pub fn probe(path: &Path) -> Result<ComPtr<IPluginFactory>, HostError> {
        let factory = platform::probe(path)?;
        Ok(factory)
    }
    
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

/// dlopen the bundle's binary (resolved per-OS by [`platform::binary_path`]).
/// Shared by [`Vst3Factory`] and the per-OS host loader.
fn open(path: &Path) -> Result<Library, HostError> {
    let binary = platform::binary_path(path).ok_or_else(|| {
        HostError::Load(format!("{}: unrecognized VST3 bundle layout", path.display()))
    })?;
    // SAFETY: loads a shared library; `binary` comes from scanning the standard
    // plugin directories.
    unsafe { Library::new(&binary) }
        .map_err(|e| HostError::Load(format!("{}: {e}", binary.display())))
}

/// Call the standard `GetPluginFactory` entry point.
fn read_factory(library: &Library) -> Result<ComPtr<IPluginFactory>, HostError> {
    // SAFETY: GetPluginFactory is the standard VST3 entry point.
    let raw = unsafe {
        let get: Symbol<unsafe extern "C" fn() -> *mut IPluginFactory> = library
            .get(b"GetPluginFactory\0")
            .map_err(|e| HostError::Load(format!("no GetPluginFactory ({e})")))?;
        get()
    };
    unsafe { ComPtr::from_raw(raw) }
        .ok_or_else(|| HostError::Load("VST3 returned a null plugin factory".into()))
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
