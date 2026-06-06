//! VST3 hosting via the `vst3` COM bindings (imported as `vst3_sdk` to avoid a
//! name clash with this module).
//!
//! [`module`] loads a bundle and gets its factory; [`probe`] enumerates the
//! audio classes for the catalog. Instantiation / processing / state / editor
//! land in later tasks (T12, T13, T30, T22).

pub mod module;
pub mod probe;
