//! The VST3 plugin editor: an `IPlugView` attached to a native window.
//!
//! Mirrors the CLAP editor: we open a [`PlatformWindow`] and hand the plugin
//! its content surface to draw into, then size the window to the view. The
//! window's [`RawWindowHandle`] variant selects the VST3 platform type, so this
//! is platform-agnostic with no `cfg` of its own.

use std::ffi::c_void;

use raw_window_handle::RawWindowHandle;
use vst3_sdk::ComPtr;
use vst3_sdk::Steinberg::Vst::{IEditController, IEditControllerTrait};
use vst3_sdk::Steinberg::{
    FIDString, IPlugView, IPlugViewTrait, ViewRect, kPlatformTypeHWND, kPlatformTypeNSView,
    kPlatformTypeX11EmbedWindowID, kResultOk, kResultTrue,
};

use crate::editor_window::{EditorWindow, PlatformWindow};
use crate::host::HostError;

/// A plugin's open editor: the COM view plus the window hosting it.
pub struct Vst3Editor {
    /// Declared before `window` so the view is released before the window
    /// (which owns the native view it's attached to) closes.
    view: ComPtr<IPlugView>,
    window: PlatformWindow,
}

impl Vst3Editor {
    /// Create the controller's editor view and host it in a native window.
    pub fn open(controller: &ComPtr<IEditController>, title: &str) -> Result<Self, HostError> {
        // SAFETY: createView returns a freshly ref-counted IPlugView or null.
        let view_ptr = unsafe { controller.createView(b"editor\0".as_ptr() as *const _) };
        let view = unsafe { ComPtr::<IPlugView>::from_raw(view_ptr) }
            .ok_or_else(|| HostError::Load("plugin has no editor view".into()))?;

        // Initial size from the plugin.
        let mut rect = ViewRect { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { view.getSize(&mut rect) };
        let width = (rect.right - rect.left).max(0) as f64;
        let height = (rect.bottom - rect.top).max(0) as f64;

        let window = PlatformWindow::open(width, height, title);
        let handle = window
            .content_handle()
            .ok_or_else(|| HostError::Load("editor window has no content handle".into()))?;
        let (parent, kind) = vst3_platform(handle)
            .ok_or_else(|| HostError::Load("unsupported window handle for VST3".into()))?;

        let supported = unsafe { view.isPlatformTypeSupported(kind) };
        if supported != kResultOk && supported != kResultTrue {
            window.close();
            return Err(HostError::Load("plugin editor does not support this window type".into()));
        }

        let attached = unsafe { view.attached(parent, kind) };
        if attached != kResultOk && attached != kResultTrue {
            window.close();
            return Err(HostError::Load(format!("IPlugView::attached failed ({attached})")));
        }

        let editor = Self { view, window };
        editor.fit();
        Ok(editor)
    }

    /// Track the plugin view's size each frame (the plugin resizes itself).
    pub fn fit(&self) {
        self.window.fit_to_plugin_view();
    }

    /// Detach the view and close the window.
    pub fn close(self) {
        unsafe { self.view.removed() };
        self.window.close();
    }
}

/// Map a window handle to the VST3 platform pointer + type string for
/// `IPlugView::attached`. Returns `None` for handle kinds VST3 can't embed.
fn vst3_platform(handle: RawWindowHandle) -> Option<(*mut c_void, FIDString)> {
    match handle {
        RawWindowHandle::AppKit(h) => Some((h.ns_view.as_ptr(), kPlatformTypeNSView)),
        RawWindowHandle::Win32(h) => Some((h.hwnd.get() as *mut c_void, kPlatformTypeHWND)),
        RawWindowHandle::Xlib(h) => {
            Some((h.window as *mut c_void, kPlatformTypeX11EmbedWindowID))
        }
        _ => None,
    }
}
