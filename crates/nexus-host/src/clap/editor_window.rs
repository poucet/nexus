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
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// A native window whose content view hosts the plugin editor.
pub struct EditorWindow {
    window: Retained<NSWindow>,
}

impl EditorWindow {
    /// Open a window of the given content size. It's resizable only if the
    /// plugin supports resizing. Returns `None` if not on the main thread.
    pub fn open(width: f64, height: f64, title: &str, resizable: bool) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let content_rect = NSRect::new(
            NSPoint::new(140.0, 140.0),
            NSSize::new(width.max(200.0), height.max(120.0)),
        );
        let mut style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;
        if resizable {
            style |= NSWindowStyleMask::Resizable;
        }

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

    /// Resize the content area to the given size.
    pub fn set_content_size(&self, width: f64, height: f64) {
        self.window
            .setContentSize(NSSize::new(width.max(1.0), height.max(1.0)));
    }

    /// Make the plugin's view (the last subview it added) fill the content view
    /// and track its size, so the editor is actually visible and follows
    /// window resizes.
    pub fn layout_plugin_view(&self) {
        let Some(content) = self.window.contentView() else { return };
        let Some(view) = content.subviews().lastObject() else { return };
        let bounds = content.bounds();
        view.setFrame(bounds);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
    }

    /// Close the window.
    pub fn close(&self) {
        self.window.close();
    }
}
