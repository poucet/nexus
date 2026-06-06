//! A native window that hosts a plugin's editor view, shared by the CLAP and
//! VST3 backends.
//!
//! **This module is the single place in the crate that branches on
//! `target_os`.** It defines one [`EditorWindow`] trait (the cross-platform
//! contract) and `cfg`-selects a concrete per-OS implementation, re-exported as
//! [`PlatformWindow`]. Consumers (the CLAP and VST3 editors) use
//! `PlatformWindow` with static dispatch and never mention a platform.
//!
//! Platforms without an implementation `compile_error!` here rather than
//! silently degrading — adding support means writing a real `EditorWindow` for
//! that OS, not a stub.
//!
//! The cross-platform seam is [`raw_window_handle`]: a window exposes its
//! content surface as a [`RawWindowHandle`], and the backends embed the plugin
//! generically (CLAP via clack's `from_window_handle`, VST3 by mapping the
//! handle variant to its platform type). Window *creation/sizing/closing* can't
//! be abstracted by a handle crate (and a windowing library like winit would
//! need to own the event loop, which collides with the app's eframe loop) — so
//! it lives here, per OS.

use raw_window_handle::RawWindowHandle;

/// The cross-platform contract for a native window hosting a plugin editor
/// view. Implemented once per supported OS in the modules below.
pub trait EditorWindow: Sized {
    /// Open a window of the given content size. Must be called on the main
    /// thread.
    fn open(width: f64, height: f64, title: &str) -> Self;

    /// The window's content surface, for a backend to parent the plugin into.
    fn content_handle(&self) -> Option<RawWindowHandle>;

    /// Track the plugin view's size: size the window to it (the plugin resizes
    /// itself). Must never resize the plugin's own view.
    fn fit_to_plugin_view(&self);

    /// Close the window.
    fn close(&self);
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacOsEditorWindow as PlatformWindow;

#[cfg(target_os = "windows")]
compile_error!(
    "editor_window: Windows is not supported yet — add a Win32 `EditorWindow` \
     impl (HWND → RawWindowHandle::Win32) and re-export it as PlatformWindow."
);

#[cfg(target_os = "linux")]
compile_error!(
    "editor_window: Linux is not supported yet — add an X11 `EditorWindow` \
     impl (X11 window → RawWindowHandle::Xlib) and re-export it as PlatformWindow."
);

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("editor_window: no EditorWindow implementation for this target OS.");
