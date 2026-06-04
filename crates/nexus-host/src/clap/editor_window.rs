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
    /// Open a window of the given content size. The window is **not**
    /// user-resizable — its size tracks the plugin's view (see
    /// [`fit_to_plugin_view`](Self::fit_to_plugin_view)); plugins resize
    /// themselves through their own controls and the window follows. Returns
    /// `None` if not on the main thread.
    pub fn open(width: f64, height: f64, title: &str) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let content_rect = NSRect::new(
            NSPoint::new(140.0, 140.0),
            NSSize::new(width.max(200.0), height.max(120.0)),
        );
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;

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

    // (window size is driven only by `fit_to_plugin_view`, below)

    /// Size the window to the plugin's view and pin that view to the bottom-left
    /// so it's fully visible. Crucially this **never resizes the plugin's view**
    /// — it only reads the view's size and moves the window/origin — so it can't
    /// fight the plugin's own sizing (which caused a resize feedback loop).
    pub fn fit_to_plugin_view(&self) {
        let Some(content) = self.window.contentView() else { return };
        let Some(view) = content.subviews().lastObject() else { return };
        let size = view.frame().size;
        if size.width < 1.0 || size.height < 1.0 {
            return;
        }
        // Pin the plugin's view to the origin (Cocoa is bottom-left) so it isn't
        // clipped above the visible area.
        view.setFrameOrigin(NSPoint::new(0.0, 0.0));
        // Match the window to the view, only if it actually differs.
        let current = content.bounds().size;
        if (current.width - size.width).abs() > 0.5 || (current.height - size.height).abs() > 0.5 {
            self.window.setContentSize(size);
        }
    }

    /// Close the window.
    pub fn close(&self) {
        self.window.close();
    }
}
