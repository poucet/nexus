//! CLAP hosting, built on the `clack` crates.
//!
//! [`host`] defines the clack host handler types Nexus presents to plugins and
//! the host-side extensions it offers. Later modules add the factory probe,
//! instantiation, processing, and per-aspect extensions
//! (see [`PLUGIN_EXTENSIONS.md`](../../../docs/designs/PLUGIN_EXTENSIONS.md)).

pub mod host;
pub mod instance;
pub mod params;
pub mod ports;
pub mod probe;
pub mod process;
