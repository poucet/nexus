//! Which plugin binaries have live instances, and on which thread the first
//! of them was made.
//!
//! **Why a host needs this.** A JUCE-built plugin on macOS takes, as its
//! message thread, the thread that made the first live object of its
//! *binary*, and keeps it for as long as any object of that binary lives
//! (every instance shares it). A [`Replica`](crate::Replica) loaded on the
//! main thread for its editor only becomes JUCE's message thread if no other
//! instance of that binary is alive elsewhere. Otherwise its editor runs on a
//! thread JUCE does not take for the message thread, and the host never hears
//! its edits.
//!
//! **The rule** (`docs/research/owning-thread-replicas.md`): a plugin's
//! editor on the main thread is safe iff [`first_thread`] of its
//! [`binary`](crate::PluginInstance::binary) is the main thread. A host gets
//! there in one of two ways:
//!
//! - when the instance is its binary's only one ([`live_instances`] is 1),
//!   drop it *before* loading its replica on the main thread, so the count
//!   passes through zero (a gap in the audio). A gapless move leaves the
//!   first thread where it was;
//! - otherwise keep every instance of that binary on the main thread, and
//!   load new ones there too.
//!
//! nexus counts only the instances it made (CLAP and VST3, by path and in
//! process). It cannot tell whether a binary is JUCE-built, so the rule
//! is for every binary, and costs nothing for a binary with one instance.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::ThreadId;

/// Per binary: how many instances are alive, and the thread that made the
/// first of them (the one that took the count from zero).
/// A panic elsewhere cannot leave the counts half-written (each change is
/// one step), so a poisoned lock is still good.
fn table() -> MutexGuard<'static, HashMap<PathBuf, (usize, ThreadId)>> {
    static TABLE: OnceLock<Mutex<HashMap<PathBuf, (usize, ThreadId)>>> = OnceLock::new();
    TABLE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// One path per binary, however it was named.
fn key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// How many instances of the plugin binary at `path` are alive in this
/// process (made by nexus), on any thread.
pub fn live_instances(path: &Path) -> usize {
    table().get(&key(path)).map_or(0, |&(n, _)| n)
}

/// The thread that made the first of the binary's live instances: JUCE's
/// message thread for that binary, if it is JUCE-built. `None` with no live
/// instance.
pub fn first_thread(path: &Path) -> Option<ThreadId> {
    table().get(&key(path)).map(|&(_, thread)| thread)
}

/// A live instance's place in the count: made with the instance (on its
/// owning thread), given back when it drops. Declared last in an instance,
/// so the plugin is gone before the count says so.
pub(crate) struct Lease {
    path: PathBuf,
}

impl Lease {
    pub(crate) fn take(path: &Path) -> Self {
        let path = key(path);
        let mut table = table();
        let entry = table.entry(path.clone()).or_insert((0, std::thread::current().id()));
        entry.0 += 1;
        Self { path }
    }

    /// The binary, as counted.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut table = table();
        if let Some(entry) = table.get_mut(&self.path) {
            entry.0 -= 1;
            if entry.0 == 0 {
                table.remove(&self.path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_thread_holds_until_the_last_instance_goes() {
        let path = Path::new("/nowhere/nexus-binary-test.clap");
        assert_eq!(live_instances(path), 0);
        let here = Lease::take(path);
        let there = std::thread::spawn(|| Lease::take(Path::new("/nowhere/nexus-binary-test.clap")))
            .join()
            .unwrap();
        assert_eq!(live_instances(path), 2);
        assert_eq!(first_thread(path), Some(std::thread::current().id()));

        drop(here); // the first goes; JUCE's message thread would stay
        assert_eq!(live_instances(path), 1);
        assert_eq!(first_thread(path), Some(std::thread::current().id()));

        drop(there);
        assert_eq!((live_instances(path), first_thread(path)), (0, None));
        let again = std::thread::spawn(|| {
            let lease = Lease::take(Path::new("/nowhere/nexus-binary-test.clap"));
            (std::thread::current().id(), first_thread(lease.path()))
        })
        .join()
        .unwrap();
        assert_eq!(Some(again.0), again.1, "a fresh count takes its own first thread");
    }
}
