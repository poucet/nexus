//! macOS VST3 loading via **`CFBundle`** — the canonical Steinberg approach.
//!
//! `CFBundleCreate` on the `.vst3` directory, `CFBundleLoadExecutable` to map
//! the binary (which also resolves the executable name for us), then the
//! standard entry points via `CFBundleGetFunctionPointerForName`. For hosting we
//! hand the plugin that same real `CFBundleRef` in `bundleEntry` so it can find
//! its resources. (Passing an `NSBundle` here crashes the plugin — NSBundle is
//! **not** toll-free bridged to CFBundle.)

use std::ffi::c_void;
use std::path::Path;
use std::ptr;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2_foundation::NSString;
use vst3_sdk::ComPtr;
use vst3_sdk::Steinberg::IPluginFactory;

use super::LoadedModule;
use crate::host::HostError;

/// An untyped CoreFoundation reference (`CFBundleRef`, `CFURLRef`, …).
type CFTypeRef = *const c_void;

/// `kCFURLPOSIXPathStyle`.
const POSIX_PATH_STYLE: isize = 0;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFBundleCreate(allocator: CFTypeRef, bundle_url: CFTypeRef) -> CFTypeRef;
    fn CFBundleLoadExecutable(bundle: CFTypeRef) -> u8;
    fn CFBundleUnloadExecutable(bundle: CFTypeRef);
    fn CFBundleGetFunctionPointerForName(bundle: CFTypeRef, name: CFTypeRef) -> *mut c_void;
    fn CFURLCreateWithFileSystemPath(
        allocator: CFTypeRef,
        path: CFTypeRef,
        style: isize,
        is_directory: u8,
    ) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
}

/// Owns the plugin's `CFBundle`, keeping its executable mapped for the lifetime
/// of any COM object obtained from it.
struct MacModule {
    bundle: CFTypeRef,
}

// SAFETY: a CFBundle is safe to retain/release/unload from any thread; this
// handle only pins it and never calls into the plugin.
unsafe impl Send for MacModule {}
unsafe impl Sync for MacModule {}

impl LoadedModule for MacModule {}

impl Drop for MacModule {
    fn drop(&mut self) {
        // SAFETY: `bundle` came from CFBundleCreate (+1) and is still valid.
        unsafe {
            CFBundleUnloadExecutable(self.bundle);
            CFRelease(self.bundle);
        }
    }
}

/// Load just to read the factory — no module init (`bundleEntry`).
pub fn probe(path: &Path) -> Result<(Box<dyn LoadedModule>, ComPtr<IPluginFactory>), HostError> {
    let (module, factory) = load(path, false)?;
    Ok((Box::new(module), factory))
}

/// Load and run the module init entry, handing the plugin its real `CFBundle`.
pub fn host(path: &Path) -> Result<(Arc<dyn LoadedModule>, ComPtr<IPluginFactory>), HostError> {
    let (module, factory) = load(path, true)?;
    Ok((Arc::new(module), factory))
}

/// Create a `CFBundle` for the `.vst3` directory, map its executable, optionally
/// run `bundleEntry` with that bundle, and read `GetPluginFactory`.
fn load(path: &Path, run_entry: bool) -> Result<(MacModule, ComPtr<IPluginFactory>), HostError> {
    let path_str = path
        .to_str()
        .ok_or_else(|| HostError::Load(format!("{}: non-UTF-8 path", path.display())))?;
    let ns_path = NSString::from_str(path_str);

    // CFURL for the bundle directory, then the bundle itself. NSString *is*
    // toll-free bridged to CFString, so its pointer is a valid CFStringRef.
    // SAFETY: CoreFoundation calls with valid arguments; the URL is released
    // once the bundle is created.
    let bundle = unsafe {
        let url = CFURLCreateWithFileSystemPath(
            ptr::null(),
            Retained::as_ptr(&ns_path) as CFTypeRef,
            POSIX_PATH_STYLE,
            1,
        );
        if url.is_null() {
            return Err(HostError::Load(format!("{}: could not form bundle URL", path.display())));
        }
        let bundle = CFBundleCreate(ptr::null(), url);
        CFRelease(url);
        bundle
    };
    if bundle.is_null() {
        return Err(HostError::Load(format!("{}: CFBundleCreate failed", path.display())));
    }
    // From here, `module` releases the bundle on any early return.
    let module = MacModule { bundle };

    // SAFETY: `bundle` is a valid CFBundleRef.
    if unsafe { CFBundleLoadExecutable(bundle) } == 0 {
        return Err(HostError::Load(format!("{}: could not load executable", path.display())));
    }

    if run_entry {
        // SAFETY: bundleEntry(CFBundleRef) -> bool is the macOS VST3 init entry;
        // we hand the plugin its own valid CFBundle.
        unsafe {
            let entry = function_ptr(bundle, "bundleEntry");
            if !entry.is_null() {
                let entry: unsafe extern "C" fn(CFTypeRef) -> bool = std::mem::transmute(entry);
                entry(bundle);
            }
        }
    }

    // SAFETY: GetPluginFactory() -> *mut IPluginFactory is the standard entry.
    let raw = unsafe {
        let get = function_ptr(bundle, "GetPluginFactory");
        if get.is_null() {
            return Err(HostError::Load(format!("{}: no GetPluginFactory", path.display())));
        }
        let get: unsafe extern "C" fn() -> *mut IPluginFactory = std::mem::transmute(get);
        get()
    };
    let factory = unsafe { ComPtr::from_raw(raw) }
        .ok_or_else(|| HostError::Load("VST3 returned a null plugin factory".into()))?;

    Ok((module, factory))
}

/// `CFBundleGetFunctionPointerForName(bundle, name)`.
///
/// # Safety
/// `bundle` must be a valid `CFBundleRef` with its executable loaded.
unsafe fn function_ptr(bundle: CFTypeRef, name: &str) -> *mut c_void {
    let name = NSString::from_str(name);
    unsafe { CFBundleGetFunctionPointerForName(bundle, Retained::as_ptr(&name) as CFTypeRef) }
}
