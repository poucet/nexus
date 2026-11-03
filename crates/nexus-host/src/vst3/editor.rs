//! The VST3 plugin editor: an `IPlugView` attached to a native view.
//!
//! Mirrors the CLAP editor: either nexus opens a [`PlatformWindow`] and hands
//! the plugin its content surface, sizing the window to the view, or the host
//! hands nexus a view of its own ([`Vst3Editor::open_in`]). The handle's
//! [`RawWindowHandle`] variant selects the VST3 platform type, so this is
//! platform-agnostic with no `cfg` of its own.
//!
//! The view is given an `IPlugFrame` ([`Vst3PlugFrame`]): a plugin that wants
//! another size calls `resizeView`, which nexus accepts (it calls the view's
//! `onSize`) and queues as a [`Notice::EditorResize`](crate::Notice::EditorResize).

use std::ffi::c_void;

use crossbeam_channel::Sender;
use raw_window_handle::RawWindowHandle;
use vst3_sdk::Steinberg::Vst::{IEditController, IEditControllerTrait};
use vst3_sdk::Steinberg::{
    FIDString, IPlugFrame, IPlugFrameTrait, IPlugView, IPlugViewTrait, ViewRect,
    kInvalidArgument, kPlatformTypeHWND, kPlatformTypeNSView, kPlatformTypeX11EmbedWindowID,
    kResultOk, kResultTrue, tresult,
};
use vst3_sdk::{Class, ComPtr, ComRef, ComWrapper};

use crate::editor_window::{EditorWindow, PlatformWindow};
use crate::host::{EditorSize, HostError};
use crate::vst3::handler::HandlerCall;

/// Nexus's `IPlugFrame`: accepts the plugin's resize and queues it for `pump`.
pub struct Vst3PlugFrame {
    calls: Sender<HandlerCall>,
}

impl Class for Vst3PlugFrame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for Vst3PlugFrame {
    unsafe fn resizeView(&self, view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        let (Some(view), false) = (unsafe { ComRef::from_raw(view) }, new_size.is_null()) else {
            return kInvalidArgument;
        };
        let size = size_of(unsafe { &*new_size });
        unsafe { view.onSize(new_size) };
        let _ = self.calls.send(HandlerCall::ResizeView(size));
        kResultTrue
    }
}

fn size_of(rect: &ViewRect) -> EditorSize {
    EditorSize {
        width: (rect.right - rect.left).max(0) as u32,
        height: (rect.bottom - rect.top).max(0) as u32,
    }
}

fn ok(result: tresult) -> bool {
    result == kResultOk || result == kResultTrue
}

/// Where an open editor lives.
enum Home {
    /// In a window nexus opened for it.
    Window(PlatformWindow),
    /// In a view the host owns.
    Parent,
}

/// A plugin's open editor: the COM view plus where it lives.
pub struct Vst3Editor {
    /// Declared before `home` so the view is released before the window
    /// (which owns the native view it's attached to) closes.
    view: ComPtr<IPlugView>,
    /// Kept alive while the view holds it.
    _frame: ComWrapper<Vst3PlugFrame>,
    home: Home,
}

/// Create the controller's editor view, with a frame that queues on `calls`.
fn create_view(
    controller: &ComPtr<IEditController>,
    calls: Sender<HandlerCall>,
) -> Result<(ComPtr<IPlugView>, ComWrapper<Vst3PlugFrame>), HostError> {
    // SAFETY: createView returns a freshly ref-counted IPlugView or null.
    let view_ptr = unsafe { controller.createView(c"editor".as_ptr()) };
    let view = unsafe { ComPtr::<IPlugView>::from_raw(view_ptr) }
        .ok_or_else(|| HostError::Load("plugin has no editor view".into()))?;
    let frame = ComWrapper::new(Vst3PlugFrame { calls });
    if let Some(ptr) = frame.to_com_ptr::<IPlugFrame>() {
        unsafe { view.setFrame(ptr.as_ptr()) };
    }
    Ok((view, frame))
}

/// Attach `view` to the native view `parent`.
fn attach(view: &ComPtr<IPlugView>, parent: RawWindowHandle) -> Result<(), HostError> {
    let (parent, kind) = vst3_platform(parent)
        .ok_or_else(|| HostError::Load("unsupported window handle for VST3".into()))?;
    if !ok(unsafe { view.isPlatformTypeSupported(kind) }) {
        return Err(HostError::Load("plugin editor does not support this window type".into()));
    }
    let attached = unsafe { view.attached(parent, kind) };
    if !ok(attached) {
        return Err(HostError::Load(format!("IPlugView::attached failed ({attached})")));
    }
    Ok(())
}

fn view_size(view: &ComPtr<IPlugView>) -> EditorSize {
    let mut rect = ViewRect { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { view.getSize(&mut rect) };
    size_of(&rect)
}

impl Vst3Editor {
    /// Create the controller's editor view and host it in a native window of
    /// nexus's own.
    pub fn open(
        controller: &ComPtr<IEditController>,
        title: &str,
        calls: Sender<HandlerCall>,
    ) -> Result<Self, HostError> {
        let (view, frame) = create_view(controller, calls)?;
        let size = view_size(&view);
        let window = PlatformWindow::open(size.width as f64, size.height as f64, title);
        let attached = window
            .content_handle()
            .ok_or_else(|| HostError::Load("editor window has no content handle".into()))
            .and_then(|parent| attach(&view, parent));
        if let Err(e) = attached {
            unsafe { view.setFrame(std::ptr::null_mut()) };
            window.close();
            return Err(e);
        }
        let editor = Self { view, _frame: frame, home: Home::Window(window) };
        editor.fit();
        Ok(editor)
    }

    /// Create the controller's editor view in `parent`, a view the host owns,
    /// and return it with its size.
    pub fn open_in(
        controller: &ComPtr<IEditController>,
        parent: RawWindowHandle,
        calls: Sender<HandlerCall>,
    ) -> Result<(Self, EditorSize), HostError> {
        let (view, frame) = create_view(controller, calls)?;
        if let Err(e) = attach(&view, parent) {
            unsafe { view.setFrame(std::ptr::null_mut()) };
            return Err(e);
        }
        let size = view_size(&view);
        Ok((Self { view, _frame: frame, home: Home::Parent }, size))
    }

    /// The view's current size.
    pub fn size(&self) -> EditorSize {
        view_size(&self.view)
    }

    /// Offer the view `size`: a resizable view takes the nearest it allows
    /// (`checkSizeConstraint`, then `onSize`); a fixed one keeps its own.
    /// Returns the size it has.
    pub fn resize(&self, size: EditorSize) -> EditorSize {
        if unsafe { self.view.canResize() } != kResultTrue {
            return self.size();
        }
        let mut rect = ViewRect { left: 0, top: 0, right: size.width as i32, bottom: size.height as i32 };
        unsafe {
            self.view.checkSizeConstraint(&mut rect);
            self.view.onSize(&mut rect);
        }
        self.fit();
        size_of(&rect)
    }

    /// Track the plugin view's size each frame (the plugin resizes itself).
    /// Returns false when the user closed nexus's window.
    pub fn fit(&self) -> bool {
        match &self.home {
            Home::Window(window) if !window.is_open() => false,
            Home::Window(window) => {
                window.fit_to_plugin_view();
                true
            }
            Home::Parent => true,
        }
    }

    /// Detach the view, and close its window if nexus opened one.
    pub fn close(self) {
        unsafe {
            self.view.removed();
            self.view.setFrame(std::ptr::null_mut());
        }
        if let Home::Window(window) = &self.home {
            window.close();
        }
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use vst3_sdk::Steinberg::{TBool, char16, int16, kResultFalse};

    use super::*;
    use crate::notice::Notice;
    use crate::vst3::handler::{HandlerQueue, Vst3ComponentHandler};
    use crate::vst3::param_changes::PendingParams;

    /// A view that is resizable down to 100×100 and records `onSize`.
    struct FakeView {
        size: Cell<(i32, i32)>,
    }

    impl Class for FakeView {
        type Interfaces = (IPlugView,);
    }

    impl IPlugViewTrait for FakeView {
        unsafe fn isPlatformTypeSupported(&self, _: FIDString) -> tresult {
            kResultTrue
        }
        unsafe fn attached(&self, _: *mut c_void, _: FIDString) -> tresult {
            kResultOk
        }
        unsafe fn removed(&self) -> tresult {
            kResultOk
        }
        unsafe fn onWheel(&self, _: f32) -> tresult {
            kResultFalse
        }
        unsafe fn onKeyDown(&self, _: char16, _: int16, _: int16) -> tresult {
            kResultFalse
        }
        unsafe fn onKeyUp(&self, _: char16, _: int16, _: int16) -> tresult {
            kResultFalse
        }
        unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
            let (w, h) = self.size.get();
            unsafe { *size = ViewRect { left: 0, top: 0, right: w, bottom: h } };
            kResultOk
        }
        unsafe fn onSize(&self, size: *mut ViewRect) -> tresult {
            let r = unsafe { &*size };
            self.size.set((r.right - r.left, r.bottom - r.top));
            kResultOk
        }
        unsafe fn onFocus(&self, _: TBool) -> tresult {
            kResultOk
        }
        unsafe fn setFrame(&self, _: *mut IPlugFrame) -> tresult {
            kResultOk
        }
        unsafe fn canResize(&self) -> tresult {
            kResultTrue
        }
        unsafe fn checkSizeConstraint(&self, rect: *mut ViewRect) -> tresult {
            let r = unsafe { &mut *rect };
            r.right = r.right.max(r.left + 100);
            r.bottom = r.bottom.max(r.top + 100);
            kResultOk
        }
    }

    /// A plugin's `resizeView` is accepted (the view gets `onSize`) and
    /// reaches the host as one notice; a host's resize is constrained by the
    /// view.
    #[test]
    fn the_editor_resizes_both_ways() {
        let (handler, calls) = Vst3ComponentHandler::new();
        let fake = ComWrapper::new(FakeView { size: Cell::new((400, 300)) });
        let view = fake.to_com_ptr::<IPlugView>().unwrap();
        let frame_object = ComWrapper::new(Vst3PlugFrame { calls: handler.calls() });
        let frame = frame_object.to_com_ptr::<IPlugFrame>().unwrap();

        let mut rect = ViewRect { left: 0, top: 0, right: 640, bottom: 480 };
        assert_eq!(unsafe { frame.resizeView(view.as_ptr(), &mut rect) }, kResultTrue);
        assert_eq!(fake.size.get(), (640, 480), "accepted");

        let mut queue = HandlerQueue::new(calls);
        let mut out = Vec::new();
        queue.drain(|_, x| x, || 0, &mut PendingParams::default(), &mut out);
        assert_eq!(out, [Notice::EditorResize(EditorSize { width: 640, height: 480 })]);

        let editor = Vst3Editor { view, _frame: frame_object, home: Home::Parent };
        assert_eq!(editor.resize(EditorSize { width: 50, height: 700 }), EditorSize { width: 100, height: 700 });
        assert_eq!(editor.size(), EditorSize { width: 100, height: 700 });
        assert!(editor.fit(), "a parented editor stays open");
        editor.close();
    }
}
