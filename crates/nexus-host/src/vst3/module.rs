//! Loading a VST3 bundle's binary and obtaining its plugin factory.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libloading::{Library, Symbol};
use vst3_sdk::ComPtr;
use vst3_sdk::Steinberg::IPluginFactory;

use crate::host::HostError;

/// A loaded VST3 module: owns the dynamic library and its plugin factory.
pub struct Vst3Module {
    /// The plugin factory. Declared *before* `library` so it is released before
    /// the library (which holds its code) drops.
    pub factory: ComPtr<IPluginFactory>,
    /// The dynamic library, shared (`Arc`) so the audio processor can pin it
    /// loaded for as long as it lives — a released COM object must never call
    /// into an unloaded dylib.
    library: Arc<Library>,
}

impl Vst3Module {
    /// Load the `.vst3` bundle at `bundle` and fetch its plugin factory.
    pub fn load(bundle: &Path) -> Result<Self, HostError> {
        let binary = binary_path(bundle).ok_or_else(|| {
            HostError::Load(format!("{}: unrecognized VST3 bundle layout", bundle.display()))
        })?;

        // SAFETY: loads a shared library and runs its initializers; `binary`
        // comes from scanning the standard plugin directories.
        let library = unsafe { Library::new(&binary) }
            .map_err(|e| HostError::Load(format!("{}: {e}", binary.display())))?;

        // macOS modules expect `bundleEntry` to run before `GetPluginFactory`.
        // We have no CFBundle to hand it; most plugins tolerate null for the
        // factory-enumeration path. (Full hosting may need a real CFBundle.)
        #[cfg(target_os = "macos")]
        unsafe {
            if let Ok(entry) =
                library.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"bundleEntry\0")
            {
                entry(std::ptr::null_mut());
            }
        }

        // SAFETY: GetPluginFactory is the standard VST3 entry point.
        let raw = unsafe {
            let get_factory: Symbol<unsafe extern "C" fn() -> *mut IPluginFactory> = library
                .get(b"GetPluginFactory\0")
                .map_err(|e| HostError::Load(format!("{}: no GetPluginFactory ({e})", binary.display())))?;
            get_factory()
        };

        let factory = unsafe { ComPtr::from_raw(raw) }
            .ok_or_else(|| HostError::Load("VST3 returned a null plugin factory".into()))?;

        Ok(Self { factory, library: Arc::new(library) })
    }

    /// A handle that keeps the underlying dylib loaded. Hand a clone to anything
    /// (e.g. the audio processor) that outlives the [`Vst3Module`] but still
    /// holds COM objects from it.
    pub fn library(&self) -> Arc<Library> {
        self.library.clone()
    }
}

/// The macOS bundle binary, in `<bundle>.vst3/Contents/MacOS/`.
///
/// Usually named after the bundle (`Foo.vst3` → `Foo`), but not always
/// (`VCV Rack 2.vst3` → `plugin.dylib`). We prefer the bundle-name match and
/// fall back to the sole binary in `Contents/MacOS`. (The canonical source is
/// `CFBundleExecutable` in `Info.plist`; this avoids a plist dependency.)
fn binary_path(bundle: &Path) -> Option<PathBuf> {
    let macos = bundle.join("Contents").join("MacOS");

    if let Some(name) = bundle.file_stem().and_then(|s| s.to_str()) {
        let candidate = macos.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // Fall back to the single file inside Contents/MacOS.
    let mut files = std::fs::read_dir(&macos)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file());
    let first = files.next()?;
    files.next().is_none().then_some(first)
}
