//! Shared by nexus-host's integration tests: the test CLAP plugin, a block
//! driver, an allocation counter, and the env-gated real plugins.
#![allow(dead_code)] // each test binary uses its own subset

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use nexus_host::process::{AudioBuffer, EventBuffer, ProcessBlock};
use nexus_host::{
    AudioConfig, AudioProcessor, Host, PluginDescriptor, PluginFormat, PluginInstance, PluginRef,
};

/// The parameter ids of `nexus.test.gain` (crates/nexus-test-plugin).
pub mod param {
    pub const VOLUME: u32 = 1;
    pub const TEMPO: u32 = 2;
    pub const PLAYING: u32 = 3;
    pub const BEATS: u32 = 4;
    pub const SAMPLE_RATE: u32 = 5;
    pub const CALLBACKS: u32 = 6;
    pub const MIDI2: u32 = 7;
}

/// `crates/nexus-test-plugin`, built once per test process into its own target
/// dir (the outer `cargo test` holds the lock on the usual one).
pub fn test_plugin() -> PluginRef {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    let path = BUILT.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target = root.join("target/test-plugin");
        let status = std::process::Command::new(env!("CARGO"))
            .current_dir(&root)
            .args(["build", "--quiet", "-p", "nexus-test-plugin", "--target-dir"])
            .arg(&target)
            .status()
            .expect("cargo runs");
        assert!(status.success(), "building crates/nexus-test-plugin failed");
        let lib = if cfg!(target_os = "macos") {
            "libnexus_test_plugin.dylib"
        } else if cfg!(windows) {
            "nexus_test_plugin.dll"
        } else {
            "libnexus_test_plugin.so"
        };
        target.join("debug").join(lib).canonicalize().expect("the test plugin was built")
    });
    PluginRef {
        format: PluginFormat::Clap,
        path: path.clone(),
        descriptor: PluginDescriptor {
            id: "nexus.test.gain".into(),
            name: None,
            vendor: None,
            version: None,
        },
    }
}

pub fn load(plugin: &PluginRef) -> Box<dyn PluginInstance> {
    Host::new().load(plugin).expect("the plugin loads")
}

pub const FRAMES: usize = 256;

/// One stereo block of `FRAMES` frames, preallocated: input all `1.0`.
pub struct Blocks {
    pub input: AudioBuffer,
    pub output: AudioBuffer,
    pub events: EventBuffer,
}

impl Blocks {
    pub fn new() -> Self {
        let mut input = AudioBuffer::new(2, FRAMES);
        for ch in 0..2 {
            input.channel_mut(ch).unwrap().fill(1.0);
        }
        Self { input, output: AudioBuffer::new(2, FRAMES), events: EventBuffer::with_capacity(64) }
    }

    /// Process one block with whatever is in `events`, then clear them.
    /// Returns the output's peak.
    pub fn run(&mut self, processor: &mut dyn AudioProcessor) -> f32 {
        self.output.clear();
        let mut block = ProcessBlock {
            frames: FRAMES,
            input: &self.input,
            output: &mut self.output,
            events: &self.events,
        };
        processor.process(&mut block);
        self.events.clear();
        let mut peak = 0.0f32;
        for ch in 0..self.output.channel_count() {
            for &s in self.output.channel(ch).unwrap() {
                peak = peak.max(s.abs());
            }
        }
        peak
    }
}

pub fn config(sample_rate: f64) -> AudioConfig {
    AudioConfig { sample_rate, max_block_frames: FRAMES }
}

/// A `#[global_allocator]` that counts allocations made on a thread while that
/// thread has armed it. Declare it in the test binary:
/// `#[global_allocator] static ALLOC: common::Counting = common::Counting;`
pub struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count();
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        count();
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        count();
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

fn count() {
    // `try_with`: the allocator runs during thread teardown too.
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

/// How many allocations `f` made on this thread. Counts Rust allocations
/// (nexus's and clack's, or anything else using the global allocator), not a
/// C++ plugin's own `malloc`.
pub fn allocations(f: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    ALLOCATIONS.load(Ordering::Relaxed) - before
}

/// The installed free plugins the env-gated tests use, when
/// `NEXUS_REAL_PLUGIN_TEST=1` and the bundle exists. Never a copy-protected
/// plugin: those can open licence dialogs.
pub fn real_plugins() -> Vec<PluginRef> {
    if std::env::var_os("NEXUS_REAL_PLUGIN_TEST").is_none() {
        eprintln!("NEXUS_REAL_PLUGIN_TEST unset: skipping the installed plugins");
        return Vec::new();
    }
    let candidates = [
        ("/Library/Audio/Plug-Ins/CLAP/TAL-Filter-2.clap", PluginFormat::Clap),
        ("/Library/Audio/Plug-Ins/VST3/TAL-Filter-2.vst3", PluginFormat::Vst3),
        ("/Library/Audio/Plug-Ins/CLAP/Surge XT Effects.clap", PluginFormat::Clap),
        ("/Library/Audio/Plug-Ins/VST3/Surge XT Effects.vst3", PluginFormat::Vst3),
    ];
    let mut out = Vec::new();
    for (path, format) in candidates {
        let path = PathBuf::from(path);
        if !path.exists() {
            eprintln!("{} not installed: skipped", path.display());
            continue;
        }
        let ids = match format {
            PluginFormat::Clap => nexus_host::clap::probe::probe(&path),
            PluginFormat::Vst3 => nexus_host::vst3::probe::probe(&path),
        };
        match ids.ok().and_then(|d| d.into_iter().next()) {
            Some(descriptor) => out.push(PluginRef { format, path, descriptor }),
            None => eprintln!("{}: probe found no plugin", path.display()),
        }
    }
    out
}
