//! Linux VST3 loading via POSIX `dlopen` / `dlsym`.
//!
//! On Linux, a VST3 bundle is typically a directory `<name>.vst3/Contents/<arch>-linux/<name>.so`
//! (e.g. `Contents/x86_64-linux/Plugin.so` or `Contents/aarch64-linux/Plugin.so`) or a standalone `.so`.
//! Entry points:
//! - `GetPluginFactory()` -> `*mut IPluginFactory`
//! - `ModuleEntry(void* sharedLibraryHandle)` -> `bool` (optional init)
//! - `ModuleExit()` -> `bool` (optional cleanup)

use std::ffi::{c_char, c_int, c_void, CString};
use std::path::{Path, PathBuf};

use vst3_sdk::ComPtr;
use vst3_sdk::Steinberg::IPluginFactory;

use super::LoadedModule;
use crate::host::HostError;

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 0;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

/// Owns the loaded library handle (`dlopen`), keeping its binary mapped for the lifetime
/// of any COM object obtained from it.
struct LinuxModule {
    handle: *mut c_void,
}

unsafe impl Send for LinuxModule {}
unsafe impl Sync for LinuxModule {}

impl LoadedModule for LinuxModule {
    fn init(&self) {
        unsafe {
            let symbol = CString::new("ModuleEntry").unwrap();
            let entry = dlsym(self.handle, symbol.as_ptr());
            if !entry.is_null() {
                let entry: unsafe extern "C" fn(*mut c_void) -> bool = std::mem::transmute(entry);
                entry(self.handle);
            }
        }
    }
}

impl Drop for LinuxModule {
    fn drop(&mut self) {
        unsafe {
            let symbol = CString::new("ModuleExit").unwrap();
            let exit = dlsym(self.handle, symbol.as_ptr());
            if !exit.is_null() {
                let exit: unsafe extern "C" fn() -> bool = std::mem::transmute(exit);
                exit();
            }
            if !self.handle.is_null() {
                dlclose(self.handle);
            }
        }
    }
}

/// Find the `.so` binary inside a `.vst3` bundle directory or return `path` if it's already a file.
fn find_so_binary(path: &Path) -> Result<PathBuf, HostError> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }

    #[cfg(target_arch = "x86_64")]
    let arch = "x86_64-linux";
    #[cfg(target_arch = "aarch64")]
    let arch = "aarch64-linux";
    #[cfg(target_arch = "x86")]
    let arch = "i386-linux";
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "x86")))]
    let arch = "x86_64-linux";

    let arch_dir = path.join("Contents").join(arch);
    if arch_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&arch_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("so")) {
                    return Ok(p);
                }
            }
        }
    }

    // Fallback: search anywhere inside path for a .so
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("so")) {
                return Ok(p);
            }
        }
    }

    Err(HostError::Load(format!(
        "{}: no .so binary found in VST3 bundle",
        path.display()
    )))
}

/// Load the `.vst3` binary and read its `GetPluginFactory` without running `ModuleEntry`.
pub fn load(path: &Path) -> Result<(Box<dyn LoadedModule>, ComPtr<IPluginFactory>), HostError> {
    let so_path = find_so_binary(path)?;
    let c_path = CString::new(so_path.to_str().ok_or_else(|| {
        HostError::Load(format!("{}: non-UTF-8 path", so_path.display()))
    })?)
    .map_err(|e| HostError::Load(format!("invalid path CString: {e}")))?;

    let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
    if handle.is_null() {
        let err_msg = unsafe {
            let err = dlerror();
            if err.is_null() {
                "unknown dlopen error".to_string()
            } else {
                std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned()
            }
        };
        return Err(HostError::Load(format!("{}: dlopen failed: {err_msg}", so_path.display())));
    }

    let symbol = CString::new("GetPluginFactory").unwrap();
    let get_factory = unsafe { dlsym(handle, symbol.as_ptr()) };
    if get_factory.is_null() {
        unsafe { dlclose(handle) };
        return Err(HostError::Load(format!("{}: no GetPluginFactory entry point", so_path.display())));
    }

    let factory_fn: unsafe extern "C" fn() -> *mut IPluginFactory =
        unsafe { std::mem::transmute(get_factory) };
    let raw = unsafe { factory_fn() };
    let factory = unsafe { ComPtr::from_raw(raw) }
        .ok_or_else(|| {
            unsafe { dlclose(handle) };
            HostError::Load("VST3 returned a null plugin factory".into())
        })?;

    let module = LinuxModule { handle };
    Ok((Box::new(module), factory))
}
