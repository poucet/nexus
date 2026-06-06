//! VST3 hosting via the `vst3` COM bindings (imported as `vst3_sdk` to avoid a
//! name clash with this module).
//!
//! [`module`] loads a bundle and gets its factory; [`probe`] enumerates the
//! audio classes for the catalog; [`instance`] instantiates and activates a
//! plugin behind the host façade, producing the [`process`] audio half driven
//! by a host [`event_list`] and an [`host_app`] context object.
//!
//! State and editor (T30, T22) land in later tasks.

pub mod event_list;
pub mod host_app;
pub mod instance;
pub mod module;
pub mod params;
pub mod probe;
pub mod process;
pub mod state;
