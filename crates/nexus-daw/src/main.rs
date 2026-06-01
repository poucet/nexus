//! # nexus-daw
//!
//! An AI-first digital audio workstation, built on top of [`nexus_host`].
//!
//! This binary is the application shell. It is currently a stub that simply
//! constructs a host so the workspace builds end to end.

use nexus_host::Host;

fn main() {
    let _host = Host::new();
    println!("nexus-daw: AI-first DAW (stub)");
}
