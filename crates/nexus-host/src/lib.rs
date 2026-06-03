//! # nexus-host
//!
//! Engine for hosting VST3 and CLAP audio plugins.
//!
//! This crate provides the format-agnostic core: plugin discovery, loading,
//! instantiation, and audio/event processing. Higher layers (a wrapper plugin,
//! a DAW) build on top of the [`Host`] abstraction defined here.
//!
//! The design intentionally hides the differences between plugin formats behind
//! a single interface so that callers never need to branch on VST3 vs. CLAP.

pub mod clap;
pub mod discovery;
pub mod host;
pub mod process;

pub use host::{AudioConfig, Editor, Host, HostError, PluginFormat, PluginInstance};
