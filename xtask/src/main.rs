//! Build automation for the Nexus plugins.
//!
//! Compiles plugin cdylibs and arranges them into the platform-specific CLAP
//! and VST3 bundle layouts, then optionally installs those bundles into the
//! current user's plugin folders.
//!
//! The tool is plugin-name-agnostic: it discovers what to bundle from
//! `cargo metadata`. Any workspace crate that declares a
//! `[package.metadata.bundle]` table *and* produces a `cdylib` is treated as a
//! plugin. Naming and identifiers come entirely from that table, so there is
//! exactly one source of truth and no hardcoded crate name here.
//!
//! This is a trimmed-down sibling of the `droplets` xtask — same platform
//! abstraction and staging-then-swap install, without the project-specific
//! frontend / type-generation / standalone steps.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "xtask", about = "Build automation for the Nexus plugins")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build and bundle the plugin(s) for the current platform.
    Build {
        #[command(flatten)]
        opts: SelectOpts,
    },
    /// Install bundled plugin(s) into the user's CLAP/VST3 folders.
    /// Runs `build` first unless `--skip-build` is passed.
    Install {
        #[arg(long)]
        skip_build: bool,
        #[command(flatten)]
        opts: SelectOpts,
    },
}

#[derive(clap::Args)]
struct SelectOpts {
    #[arg(long, short, default_value = "release")]
    profile: Profile,
    #[arg(long, short, default_value = "both")]
    format: Format,
    /// Limit to a single plugin crate by package name. Defaults to every
    /// bundleable crate in the workspace.
    #[arg(long)]
    package: Option<String>,
}

#[derive(ValueEnum, Clone, Copy)]
enum Profile {
    Debug,
    Release,
}

impl Profile {
    fn as_str(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

#[derive(ValueEnum, Clone, Copy, PartialEq, Eq)]
enum Format {
    Clap,
    Vst3,
    Both,
}

impl Format {
    fn wants_clap(self) -> bool {
        matches!(self, Format::Clap | Format::Both)
    }
    fn wants_vst3(self) -> bool {
        matches!(self, Format::Vst3 | Format::Both)
    }
}

// ---------------------------------------------------------------------------
// Host platform abstraction — keeps build/install symmetric across OSes.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum HostOs {
    Macos,
    Linux,
    Windows,
}

impl HostOs {
    fn detect() -> anyhow::Result<Self> {
        match env::consts::OS {
            "macos" => Ok(HostOs::Macos),
            "linux" => Ok(HostOs::Linux),
            "windows" => Ok(HostOs::Windows),
            other => Err(anyhow::anyhow!("unsupported host OS: {other}")),
        }
    }

    /// File name cargo emits for a cdylib whose crate name is `lib_name`,
    /// e.g. `libnexus_wrap.dylib`.
    fn dylib_filename(self, lib_name: &str) -> String {
        let stem = lib_name.replace('-', "_");
        match self {
            HostOs::Macos => format!("lib{stem}.dylib"),
            HostOs::Linux => format!("lib{stem}.so"),
            HostOs::Windows => format!("{stem}.dll"),
        }
    }

    /// VST3 subdir name inside the bundle's `Contents/` dir (per VST3 spec).
    fn vst3_arch_subdir(self) -> String {
        match (self, env::consts::ARCH) {
            (HostOs::Macos, _) => "MacOS".to_string(),
            (HostOs::Linux, arch) => format!("{arch}-linux"),
            (HostOs::Windows, arch) => format!("{arch}-win"),
        }
    }

    /// User-scope install directories, as `(clap_dir, vst3_dir)`.
    fn user_plugin_dirs(self) -> anyhow::Result<(PathBuf, PathBuf)> {
        match self {
            HostOs::Macos => {
                let home = env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
                let base = PathBuf::from(home).join("Library/Audio/Plug-Ins");
                Ok((base.join("CLAP"), base.join("VST3")))
            }
            HostOs::Linux => {
                let home = env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
                let base = PathBuf::from(home);
                Ok((base.join(".clap"), base.join(".vst3")))
            }
            HostOs::Windows => {
                let appdata = env::var("LOCALAPPDATA")
                    .or_else(|_| env::var("APPDATA"))
                    .map_err(|_| anyhow::anyhow!("LOCALAPPDATA/APPDATA not set"))?;
                let base = PathBuf::from(appdata).join("Programs").join("Common");
                Ok((base.join("CLAP"), base.join("VST3")))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin discovery — every bundleable crate, learned from `cargo metadata`.
// ---------------------------------------------------------------------------

/// A workspace crate that can be bundled: it has a cdylib target and a
/// `[package.metadata.bundle]` table.
#[derive(Debug, Clone)]
struct Plugin {
    /// Cargo package name, e.g. `nexus-wrap` (passed to `cargo build -p`).
    package: String,
    /// cdylib crate name, used to compute the built dylib filename.
    lib_name: String,
    bundle: BundleInfo,
}

#[derive(Debug, Clone)]
struct BundleInfo {
    display_name: String,
    version: String,
    clap_bundle_id: String,
    vst3_bundle_id: String,
}

impl BundleInfo {
    fn clap_bundle_name(&self) -> String {
        format!("{}.clap", self.display_name)
    }
    fn vst3_bundle_name(&self) -> String {
        format!("{}.vst3", self.display_name)
    }
}

/// Discover bundleable plugins. `only` optionally restricts to one package.
fn discover_plugins(project_root: &Path, only: Option<&str>) -> anyhow::Result<Vec<Plugin>> {
    let output = Command::new(cargo_cmd())
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(project_root)
        .output()?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    #[derive(serde::Deserialize)]
    struct Metadata {
        packages: Vec<Package>,
    }
    #[derive(serde::Deserialize)]
    struct Package {
        name: String,
        version: String,
        targets: Vec<Target>,
        metadata: Option<serde_json::Value>,
    }
    #[derive(serde::Deserialize)]
    struct Target {
        name: String,
        kind: Vec<String>,
    }

    let meta: Metadata = serde_json::from_slice(&output.stdout)?;

    let mut plugins = Vec::new();
    for pkg in meta.packages {
        if let Some(name) = only {
            if pkg.name != name {
                continue;
            }
        }

        let Some(bundle) = pkg.metadata.as_ref().and_then(|m| m.get("bundle")) else {
            continue; // not a plugin — no bundle metadata
        };
        let Some(lib_target) = pkg.targets.iter().find(|t| t.kind.iter().any(|k| k == "cdylib"))
        else {
            return Err(anyhow::anyhow!(
                "{} declares [package.metadata.bundle] but has no cdylib target",
                pkg.name
            ));
        };

        let get_str = |key: &str| -> anyhow::Result<String> {
            bundle
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| {
                    anyhow::anyhow!("{}: [package.metadata.bundle].{key} missing or non-string", pkg.name)
                })
        };

        plugins.push(Plugin {
            package: pkg.name.clone(),
            lib_name: lib_target.name.clone(),
            bundle: BundleInfo {
                display_name: get_str("display_name")?,
                version: pkg.version,
                clap_bundle_id: get_str("clap_bundle_id")?,
                vst3_bundle_id: get_str("vst3_bundle_id")?,
            },
        });
    }

    if plugins.is_empty() {
        return Err(match only {
            Some(name) => anyhow::anyhow!("no bundleable crate named '{name}' (does it have a cdylib target and [package.metadata.bundle]?)"),
            None => anyhow::anyhow!("no bundleable crates found (none declare [package.metadata.bundle])"),
        });
    }
    Ok(plugins)
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Commands::Build { opts } => build(&opts),
        Commands::Install { skip_build, opts } => {
            if !skip_build {
                build(&opts)?;
            }
            install(&opts)
        }
    }
}

fn cargo_cmd() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn project_root() -> anyhow::Result<PathBuf> {
    let current = env::current_dir()?;
    Ok(if current.file_name().and_then(|n| n.to_str()) == Some("xtask") {
        current.parent().unwrap().to_path_buf()
    } else {
        current
    })
}

// ---------------------------------------------------------------------------
// build + bundle
// ---------------------------------------------------------------------------

fn build(opts: &SelectOpts) -> anyhow::Result<()> {
    let host = HostOs::detect()?;
    let project_root = project_root()?;
    let plugins = discover_plugins(&project_root, opts.package.as_deref())?;

    compile(&plugins, opts.profile)?;

    let bundle_dir = project_root.join("target").join("bundle");
    fs::create_dir_all(&bundle_dir)?;

    for plugin in &plugins {
        let dylib_src = project_root
            .join("target")
            .join(opts.profile.as_str())
            .join(host.dylib_filename(&plugin.lib_name));
        if !dylib_src.exists() {
            return Err(anyhow::anyhow!(
                "built dylib not found at {dylib_src:?} — cargo build may have failed"
            ));
        }

        if opts.format.wants_clap() {
            create_clap_bundle(host, &plugin.bundle, &dylib_src, &bundle_dir)?;
        }
        if opts.format.wants_vst3() {
            create_vst3_bundle(host, &plugin.bundle, &dylib_src, &bundle_dir)?;
        }
    }
    Ok(())
}

/// Compile all selected plugin packages in a single cargo invocation.
fn compile(plugins: &[Plugin], profile: Profile) -> anyhow::Result<()> {
    let names: Vec<&str> = plugins.iter().map(|p| p.package.as_str()).collect();
    println!("Building: {}", names.join(", "));

    let mut cmd = Command::new(cargo_cmd());
    cmd.arg("build").current_dir(project_root()?);
    for name in &names {
        cmd.args(["--package", name]);
    }
    if matches!(profile, Profile::Release) {
        cmd.arg("--release");
    }

    // VST3 export via clap-wrapper needs the Steinberg VST3 SDK. The CLAP side
    // builds fine without it.
    match env::var("CLAP_WRAPPER_VST3_SDK") {
        Ok(sdk) => {
            println!("Using VST3 SDK at: {sdk}");
            cmd.env("CLAP_WRAPPER_VST3_SDK", sdk);
        }
        Err(_) => {
            println!("Note: CLAP_WRAPPER_VST3_SDK not set — VST3 export may not work yet.");
        }
    }

    if !cmd.status()?.success() {
        return Err(anyhow::anyhow!("cargo build failed"));
    }
    Ok(())
}

/// CLAP bundle layout:
///   macOS:   <Name>.clap/Contents/MacOS/<Name>
///   Linux:   <Name>.clap   (flat .so)
///   Windows: <Name>.clap   (flat .dll)
fn create_clap_bundle(
    host: HostOs,
    info: &BundleInfo,
    dylib_src: &Path,
    bundle_dir: &Path,
) -> anyhow::Result<()> {
    let bundle_path = bundle_dir.join(info.clap_bundle_name());
    remove_any(&bundle_path)?;

    match host {
        HostOs::Macos => write_macos_bundle(&bundle_path, dylib_src, info, &info.clap_bundle_id)?,
        HostOs::Linux | HostOs::Windows => {
            fs::copy(dylib_src, &bundle_path)?;
        }
    }
    println!("Created CLAP bundle at: {}", bundle_path.display());
    Ok(())
}

/// VST3 bundle layout (a bundle directory on every platform, per the spec):
///   macOS:   <Name>.vst3/Contents/MacOS/<Name>
///   Linux:   <Name>.vst3/Contents/<arch>-linux/<Name>.so
///   Windows: <Name>.vst3/Contents/<arch>-win/<Name>.vst3
fn create_vst3_bundle(
    host: HostOs,
    info: &BundleInfo,
    dylib_src: &Path,
    bundle_dir: &Path,
) -> anyhow::Result<()> {
    let bundle_path = bundle_dir.join(info.vst3_bundle_name());
    remove_any(&bundle_path)?;

    match host {
        HostOs::Macos => write_macos_bundle(&bundle_path, dylib_src, info, &info.vst3_bundle_id)?,
        HostOs::Linux => {
            let arch_dir = bundle_path.join("Contents").join(host.vst3_arch_subdir());
            fs::create_dir_all(&arch_dir)?;
            fs::copy(dylib_src, arch_dir.join(format!("{}.so", info.display_name)))?;
        }
        HostOs::Windows => {
            let arch_dir = bundle_path.join("Contents").join(host.vst3_arch_subdir());
            fs::create_dir_all(&arch_dir)?;
            fs::copy(dylib_src, arch_dir.join(format!("{}.vst3", info.display_name)))?;
        }
    }
    println!("Created VST3 bundle at: {}", bundle_path.display());
    Ok(())
}

/// macOS bundle builder, shared by CLAP + VST3. Writes `Contents/MacOS/<exe>`,
/// an Info.plist and PkgInfo, fixes the dylib install-name so the bundle is
/// self-contained, and applies an ad-hoc codesign so DAWs will load it.
fn write_macos_bundle(
    bundle_path: &Path,
    dylib_src: &Path,
    info: &BundleInfo,
    bundle_id: &str,
) -> anyhow::Result<()> {
    let contents = bundle_path.join("Contents");
    let macos = contents.join("MacOS");
    fs::create_dir_all(&macos)?;

    let exe = macos.join(&info.display_name);
    fs::copy(dylib_src, &exe)?;
    fix_dylib_paths(&exe, dylib_src);

    fs::write(contents.join("Info.plist"), info_plist_xml(info, bundle_id))?;
    fs::write(contents.join("PkgInfo"), "BNDL????")?;

    match Command::new("codesign")
        .args(["-s", "-", bundle_path.to_str().unwrap()])
        .status()
    {
        Ok(s) if !s.success() => println!("Warning: codesign returned non-zero"),
        Err(_) => println!("Warning: codesign not available, bundle not signed"),
        _ => {}
    }
    Ok(())
}

fn info_plist_xml(info: &BundleInfo, bundle_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>English</string>
    <key>CFBundleExecutable</key>
    <string>{name}</string>
    <key>CFBundleIdentifier</key>
    <string>{bundle_id}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{name}</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
</dict>
</plist>"#,
        name = info.display_name,
        bundle_id = bundle_id,
        version = info.version,
    )
}

fn fix_dylib_paths(dylib_path: &Path, original_dylib: &Path) {
    let new_id = format!(
        "@loader_path/{}",
        dylib_path.file_name().unwrap().to_str().unwrap()
    );
    let _ = Command::new("install_name_tool")
        .args(["-id", &new_id, dylib_path.to_str().unwrap()])
        .status();
    let _ = Command::new("install_name_tool")
        .args([
            "-change",
            original_dylib.to_str().unwrap(),
            &new_id,
            dylib_path.to_str().unwrap(),
        ])
        .status();
}

// ---------------------------------------------------------------------------
// install — staging-then-swap so a failed copy never breaks an existing install
// ---------------------------------------------------------------------------

fn install(opts: &SelectOpts) -> anyhow::Result<()> {
    let host = HostOs::detect()?;
    let project_root = project_root()?;
    let plugins = discover_plugins(&project_root, opts.package.as_deref())?;
    let bundle_dir = project_root.join("target").join("bundle");
    let (clap_user_dir, vst3_user_dir) = host.user_plugin_dirs()?;

    for plugin in &plugins {
        let info = &plugin.bundle;
        if opts.format.wants_clap() {
            let name = info.clap_bundle_name();
            install_artifact(&bundle_dir.join(&name), &clap_user_dir, &name, "CLAP")?;
        }
        if opts.format.wants_vst3() {
            let name = info.vst3_bundle_name();
            install_artifact(&bundle_dir.join(&name), &vst3_user_dir, &name, "VST3")?;
        }
    }

    println!("\n✨ Installation complete. Restart your DAW and rescan plugins.");
    Ok(())
}

fn install_artifact(src: &Path, dest_dir: &Path, bundle_name: &str, kind: &str) -> anyhow::Result<()> {
    if !src.exists() {
        return Err(anyhow::anyhow!(
            "{kind} bundle not found at {src:?}. Run `cargo bundle` first."
        ));
    }
    fs::create_dir_all(dest_dir)?;

    let dest = dest_dir.join(bundle_name);
    let staged = dest_dir.join(format!("{bundle_name}.new"));
    remove_any(&staged)?;

    // Copy into a staging path first; the old install stays intact if this fails.
    if let Err(e) = copy_any(src, &staged) {
        let _ = remove_any(&staged);
        return Err(e);
    }

    if dest.exists() {
        println!("🗑️  Removing existing {kind} → {}", dest.display());
        remove_any(&dest)?;
    }
    fs::rename(&staged, &dest)?;
    println!("✅ Installed {kind} → {}", dest.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// fs helpers
// ---------------------------------------------------------------------------

fn remove_any(path: &Path) -> anyhow::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn copy_any(src: &Path, dest: &Path) -> anyhow::Result<()> {
    if src.is_dir() {
        copy_dir_recursive(src, dest)
    } else {
        fs::copy(src, dest)?;
        Ok(())
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}
