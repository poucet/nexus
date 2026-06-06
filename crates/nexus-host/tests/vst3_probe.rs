//! Headless verification of VST3 discovery against the real plugins installed
//! on this machine. Ignored by default (machine-specific + loads binaries).
//!
//!   cargo test -p nexus-host --test vst3_probe -- --ignored --nocapture

use std::path::PathBuf;

use nexus_host::vst3;

/// Every `.vst3` bundle in the standard macOS search paths.
fn installed_vst3() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3"));
    }
    let mut bundles = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "vst3") {
                bundles.push(path);
            }
        }
    }
    bundles.sort();
    bundles
}

#[test]
#[ignore = "loads real VST3 binaries; machine-specific"]
fn probes_installed_vst3() {
    let bundles = installed_vst3();
    assert!(!bundles.is_empty(), "no VST3 plugins installed to test against");

    let (mut ok, mut empty, mut failed) = (0, 0, 0);
    for bundle in &bundles {
        let name = bundle.file_name().unwrap().to_string_lossy();
        match vst3::probe::probe(bundle) {
            Ok(descriptors) if descriptors.is_empty() => {
                empty += 1;
                println!("  EMPTY  {name}");
            }
            Ok(descriptors) => {
                ok += 1;
                for d in &descriptors {
                    println!(
                        "  OK     {name}: {} [{}]",
                        d.name.as_deref().unwrap_or("?"),
                        d.vendor.as_deref().unwrap_or("?"),
                    );
                }
            }
            Err(e) => {
                failed += 1;
                println!("  FAIL   {name}: {e}");
            }
        }
    }
    println!(
        "\n{} bundles: {ok} probed, {empty} empty, {failed} failed",
        bundles.len()
    );
    assert!(ok > 0, "expected at least one VST3 to probe successfully");
}
