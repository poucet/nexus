//! In-memory snapshot store + on-disk corpus export.
//!
//! Snapshots are the investigation loop's working set (snapshot → tweak →
//! diff) and the undo mechanism (every applying write auto-snapshots first).
//! Exported snapshots build the per-plugin **state corpus** on disk — the raw
//! material codec development and codec regression tests feed on, so it must
//! survive app restarts.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ToolError;
use crate::types::SnapshotInfo;

/// Caps: beyond these the least-recently-used snapshot is evicted.
const MAX_SNAPSHOTS: usize = 32;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
pub struct Snapshot {
    pub info: SnapshotInfo,
    pub data: Arc<Vec<u8>>,
}

#[derive(Default)]
struct Inner {
    map: HashMap<String, Snapshot>,
    /// Least-recently-used first.
    order: Vec<String>,
    total_bytes: u64,
    counter: u64,
}

#[derive(Default)]
pub struct SnapshotStore {
    inner: Mutex<Inner>,
}

impl SnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a snapshot; auto-names it "snap-N" when `name` is None. Returns
    /// the stored info (name resolved).
    pub fn insert(
        &self,
        name: Option<String>,
        note: Option<String>,
        plugin_id: String,
        data: Arc<Vec<u8>>,
        sha256: String,
    ) -> SnapshotInfo {
        let mut inner = self.inner.lock().unwrap();
        inner.counter += 1;
        let name = name.unwrap_or_else(|| format!("snap-{}", inner.counter));
        let info = SnapshotInfo {
            name: name.clone(),
            plugin_id,
            size_bytes: data.len() as u64,
            sha256,
            note,
            created_at_unix: unix_now(),
            exported_path: None,
        };
        // Replace any snapshot of the same name.
        if let Some(old) = inner.map.remove(&name) {
            inner.total_bytes -= old.info.size_bytes;
            inner.order.retain(|n| n != &name);
        }
        inner.total_bytes += info.size_bytes;
        inner.order.push(name.clone());
        inner.map.insert(name, Snapshot { info: info.clone(), data });
        // Evict LRU while over caps (never the one just inserted).
        while inner.map.len() > MAX_SNAPSHOTS
            || (inner.total_bytes > MAX_TOTAL_BYTES && inner.map.len() > 1)
        {
            let Some(evict) = inner.order.first().cloned() else { break };
            if evict == info.name {
                break;
            }
            if let Some(old) = inner.map.remove(&evict) {
                inner.total_bytes -= old.info.size_bytes;
                log::warn!("snapshot store over capacity — evicted {evict:?}");
            }
            inner.order.retain(|n| n != &evict);
        }
        info
    }

    pub fn get(&self, name: &str) -> Result<Snapshot, ToolError> {
        let mut inner = self.inner.lock().unwrap();
        let snapshot = inner.map.get(name).cloned().ok_or_else(|| {
            ToolError::not_found(format!(
                "no snapshot named {name:?} — list_snapshots shows what exists"
            ))
        })?;
        // Touch for LRU.
        inner.order.retain(|n| n != name);
        inner.order.push(name.to_string());
        Ok(snapshot)
    }

    pub fn list(&self) -> Vec<SnapshotInfo> {
        let inner = self.inner.lock().unwrap();
        let mut infos: Vec<_> = inner.map.values().map(|s| s.info.clone()).collect();
        infos.sort_by_key(|i| i.created_at_unix);
        infos
    }

    /// Export a stored snapshot to the corpus directory, recording the path.
    pub fn export(&self, name: &str) -> Result<PathBuf, ToolError> {
        let snapshot = self.get(name)?;
        let dir = corpus_directory().join(sanitize(&snapshot.info.plugin_id));
        std::fs::create_dir_all(&dir)
            .map_err(|e| ToolError::internal(format!("create corpus dir: {e}")))?;
        let base = sanitize(&snapshot.info.name);
        let state_path = dir.join(format!("{base}.state"));
        std::fs::write(&state_path, snapshot.data.as_slice())
            .map_err(|e| ToolError::internal(format!("write corpus state: {e}")))?;
        let sidecar = serde_json::json!({
            "plugin_id": snapshot.info.plugin_id,
            "sha256": snapshot.info.sha256,
            "size_bytes": snapshot.info.size_bytes,
            "note": snapshot.info.note,
            "created_at_unix": snapshot.info.created_at_unix,
        });
        let _ = std::fs::write(
            dir.join(format!("{base}.json")),
            serde_json::to_vec_pretty(&sidecar).unwrap_or_default(),
        );
        let mut inner = self.inner.lock().unwrap();
        if let Some(stored) = inner.map.get_mut(name) {
            stored.info.exported_path = Some(state_path.display().to_string());
        }
        Ok(state_path)
    }
}

/// Where exported state blobs live: `.../Nexus/state-corpus/<plugin_id>/`.
pub fn corpus_directory() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/Nexus/state-corpus")
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Filesystem-safe name: keep alphanumerics and `-._`, replace the rest.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_') { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(names: &[&str]) -> SnapshotStore {
        let store = SnapshotStore::new();
        for name in names {
            store.insert(
                Some((*name).into()),
                None,
                "test.plugin".into(),
                Arc::new(vec![0u8; 8]),
                "00".into(),
            );
        }
        store
    }

    #[test]
    fn auto_names_and_lists_in_creation_order() {
        let store = SnapshotStore::new();
        let a = store.insert(None, None, "p".into(), Arc::new(vec![1]), "aa".into());
        let b = store.insert(None, None, "p".into(), Arc::new(vec![2]), "bb".into());
        assert_eq!(a.name, "snap-1");
        assert_eq!(b.name, "snap-2");
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn get_unknown_is_not_found() {
        let store = store_with(&["x"]);
        assert!(store.get("y").is_err());
        assert!(store.get("x").is_ok());
    }

    #[test]
    fn same_name_replaces() {
        let store = SnapshotStore::new();
        store.insert(Some("a".into()), None, "p".into(), Arc::new(vec![1, 2]), "v1".into());
        store.insert(Some("a".into()), None, "p".into(), Arc::new(vec![3]), "v2".into());
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get("a").unwrap().info.sha256, "v2");
    }

    #[test]
    fn evicts_least_recently_used_over_count_cap() {
        let store = SnapshotStore::new();
        for i in 0..MAX_SNAPSHOTS + 2 {
            store.insert(Some(format!("s{i}")), None, "p".into(), Arc::new(vec![0]), "h".into());
        }
        assert_eq!(store.list().len(), MAX_SNAPSHOTS);
        assert!(store.get("s0").is_err(), "oldest should be evicted");
    }
}
