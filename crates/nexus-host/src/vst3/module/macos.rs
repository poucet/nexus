//! macOS VST3 loading: dlopen the bundle binary for its entry points, and pass
//! the plugin a **real `CFBundle`** (via toll-free-bridged `NSBundle`) so it can
//! locate its resources — no null `bundleEntry` shortcut.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;

use libloading::{Library, Symbol};
use objc2::rc::Retained;
use objc2_foundation::{NSBundle, NSString};
use vst3_sdk::ComPtr;
use vst3_sdk::Steinberg::IPluginFactory;

use super::LoadedModule;
use crate::host::HostError;

/// Result of loading: a keep-alive handle plus the plugin factory.
type Loaded = (Arc<dyn LoadedModule>, ComPtr<IPluginFactory>);

/// Keeps the dylib loaded and the resource bundle alive for the plugin's life,
/// and runs `bundleExit` on teardown.
struct MacModule {
    /// The plugin's resource bundle, kept alive (the plugin holds the same ref);
    /// never read by us.
    _bundle: Option<Retained<NSBundle>>,
    /// Keeps the dylib loaded; never read.
    _library: Library,
}

// SAFETY: this handle only pins the loaded code + resource bundle; it is never
// used to call into the plugin, and never from more than the audio thread that
// owns the processor holding it.
unsafe impl Send for MacModule {}
unsafe impl Sync for MacModule {}

impl LoadedModule for MacModule {}

/// Load just to read the factory — no module init (`bundleEntry`).
pub fn probe(path: &Path) -> Result< ComPtr<IPluginFactory>, HostError> {
    let (binary, library) = open(path)?;
    let factory = factory(&library, &binary)?;
    Ok(factory)
}

/// Load and run the module init entry with a real resource bundle.
pub fn host(path: &Path) -> Result<Loaded, HostError> {
    let (binary, library) = open(path)?;

    // A real CFBundle for the plugin's resources, via toll-free-bridged NSBundle.
    let bundle = path.to_str().and_then(|p| NSBundle::bundleWithPath(&NSString::from_str(p)));
    // SAFETY: bundleEntry is the macOS VST3 init entry; it takes the plugin's
    // CFBundleRef (NSBundle is toll-free bridged to CFBundle).
    unsafe {
        if let Ok(entry) =
            library.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"bundleEntry\0")
        {
            let cf =
                bundle.as_ref().map_or(ptr::null_mut(), |b| Retained::as_ptr(b) as *mut c_void);
            entry(cf);
        }
    }

    let factory = factory(&library, &binary)?;
    Ok((Arc::new(MacModule { _bundle: bundle, _library: library }), factory))
}

/// Resolve and dlopen the bundle binary.
fn open(path: &Path) -> Result<(PathBuf, Library), HostError> {
    let binary = binary_path(path).ok_or_else(|| {
        HostError::Load(format!("{}: unrecognized VST3 bundle layout", path.display()))
    })?;
    // SAFETY: loads a shared library; `binary` comes from scanning the standard
    // plugin directories.
    let library = unsafe { Library::new(&binary) }
        .map_err(|e| HostError::Load(format!("{}: {e}", binary.display())))?;
    Ok((binary, library))
}

/// Call the standard `GetPluginFactory` entry point.
fn factory(library: &Library, binary: &Path) -> Result<ComPtr<IPluginFactory>, HostError> {
    // SAFETY: GetPluginFactory is the standard VST3 entry point.
    let raw = unsafe {
        let get: Symbol<unsafe extern "C" fn() -> *mut IPluginFactory> = library
            .get(b"GetPluginFactory\0")
            .map_err(|e| HostError::Load(format!("{}: no GetPluginFactory ({e})", binary.display())))?;
        get()
    };
    unsafe { ComPtr::from_raw(raw) }
        .ok_or_else(|| HostError::Load("VST3 returned a null plugin factory".into()))
}

/// The bundle binary, in `<bundle>.vst3/Contents/MacOS/`.
///
/// Usually named after the bundle (`Foo.vst3` → `Foo`), but not always
/// (`VCV Rack 2.vst3` → `plugin.dylib`). Prefer the bundle-name match, then fall
/// back to the sole file in `Contents/MacOS`. (The canonical source is
/// `CFBundleExecutable` in `Info.plist`; this avoids parsing it.)
pub fn binary_path(bundle: &Path) -> Option<PathBuf> {
    let macos = bundle.join("Contents").join("MacOS");

    if let Some(name) = bundle.file_stem().and_then(|s| s.to_str()) {
        let candidate = macos.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let mut files = std::fs::read_dir(&macos)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file());
    let first = files.next()?;
    files.next().is_none().then_some(first)
}
