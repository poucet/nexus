//! A dedicated macOS window to host a plugin's editor view.
//!
//! Plugins that only support *embedded* GUIs need a parent window. Rather than
//! cram the plugin's view into the egui canvas (where it overlaps the
//! controls), we create our own `NSWindow` and hand the plugin its content
//! view. The window lives on the shared main run loop, so it gets events
//! alongside the egui window.

use std::ffi::c_void;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// A native window whose content view hosts the plugin editor.
pub struct EditorWindow {
    window: Retained<NSWindow>,
}

impl EditorWindow {
    /// Open a titled, closable, resizable window of the given content size.
    /// Returns `None` if not called on the main thread.
    pub fn open(width: f64, height: f64, title: &str) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let content_rect = NSRect::new(
            NSPoint::new(140.0, 140.0),
            NSSize::new(width.max(200.0), height.max(120.0)),
        );
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;

        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                content_rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };

        unsafe {
            window.setTitle(&NSString::from_str(title));
            // We own the window via `Retained`; don't let Cocoa free it on close.
            window.setReleasedWhenClosed(false);
            window.center();
            window.makeKeyAndOrderFront(None);
        }

        Some(Self { window })
    }

    /// Pointer to the window's content `NSView`, for the plugin to parent into.
    pub fn content_view_ptr(&self) -> *mut c_void {
        match self.window.contentView() {
            Some(view) => Retained::as_ptr(&view) as *mut c_void,
            None => std::ptr::null_mut(),
        }
    }

    /// Close the window.
    pub fn close(&self) {
        self.window.close();
    }
}
