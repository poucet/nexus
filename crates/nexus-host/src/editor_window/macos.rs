//! macOS editor window: a native `NSWindow` whose content view hosts the
//! plugin editor, exposed to the backends as an `AppKit` [`RawWindowHandle`].

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{AppKitWindowHandle, RawWindowHandle};

use super::EditorWindow;

/// A native window whose content view hosts the plugin editor. The window is
/// **not** user-resizable — its size tracks the plugin's view (see
/// [`fit_to_plugin_view`](EditorWindow::fit_to_plugin_view)); plugins resize
/// themselves through their own controls and the window follows.
pub struct MacOsEditorWindow {
    window: Retained<NSWindow>,
}

impl EditorWindow for MacOsEditorWindow {
    fn open(width: f64, height: f64, title: &str) -> Self {
        let mtm = MainThreadMarker::new()
            .expect("editor windows must be opened on the main thread");
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

        Self { window }
    }

    fn content_handle(&self) -> Option<RawWindowHandle> {
        let view = self.window.contentView()?;
        let ptr = NonNull::new(Retained::as_ptr(&view) as *mut c_void)?;
        Some(RawWindowHandle::AppKit(AppKitWindowHandle::new(ptr)))
    }

    fn fit_to_plugin_view(&self) {
        let Some(content) = self.window.contentView() else { return };
        let Some(view) = content.subviews().lastObject() else { return };
        let size = view.frame().size;
        if size.width < 1.0 || size.height < 1.0 {
            return;
        }
        // Pin the plugin's view to the origin (Cocoa is bottom-left) so it isn't
        // clipped above the visible area. Only when it moved — setFrameOrigin
        // invalidates the view, and this runs every frame.
        let origin = view.frame().origin;
        if origin.x != 0.0 || origin.y != 0.0 {
            view.setFrameOrigin(NSPoint::new(0.0, 0.0));
        }
        // Match the window to the view, only if it actually differs.
        let current = content.bounds().size;
        if (current.width - size.width).abs() > 0.5 || (current.height - size.height).abs() > 0.5 {
            self.window.setContentSize(size);
        }
    }

    fn close(&self) {
        self.window.close();
    }
}
