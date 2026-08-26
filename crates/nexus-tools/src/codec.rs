//! `StateCodec`: the extensibility seam for per-plugin state (de)serialization.
//!
//! Every plugin stores state differently (Vital = JSON, Arturia = XML-ish,
//! Serum 2 = binary chunks, most unknown). A codec turns a plugin's opaque blob
//! into a structured JSON document (and, when the format is understood, back).
//! The registry picks the highest-confidence codec; `generic` (sniffing,
//! decode-only) is the fallback for plugins nobody has studied yet.
//!
//! Edit application (JSON-pointer ops on the document) is SHARED code here —
//! codecs only (de)serialize.

use std::sync::Arc;

use nexus_host::host::PluginDescriptor;
use serde_json::Value;

use crate::error::ToolError;
use crate::types::{CodecInfo, EditOp, StateEdit};

pub trait StateCodec: Send + Sync {
    fn id(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// Confidence this codec understands (plugin, blob): 0 = not mine,
    /// 1 = generic fallback, 255 = authored for exactly this plugin.
    fn confidence(&self, plugin: Option<&PluginDescriptor>, blob: &[u8]) -> u8;
    /// Blob → structured document. Lossless by construction: regions the codec
    /// can't interpret appear as binary-leaf descriptions, not silence.
    fn decode(&self, plugin: Option<&PluginDescriptor>, blob: &[u8]) -> Result<Value, ToolError>;
    /// Document → blob. `None` = decode-only codec.
    fn encode(&self, plugin: Option<&PluginDescriptor>, doc: &Value) -> Option<Result<Vec<u8>, ToolError>>;

    fn encode_capable(&self) -> bool {
        self.encode(None, &Value::Null).is_some()
    }

    fn info(&self) -> CodecInfo {
        CodecInfo {
            id: self.id().to_string(),
            description: self.description().to_string(),
            encode_capable: self.encode_capable(),
        }
    }
}

/// Ordered codec collection; selection = highest confidence, first wins ties.
pub struct CodecRegistry {
    codecs: Vec<Arc<dyn StateCodec>>,
}

impl CodecRegistry {
    pub fn new(codecs: Vec<Arc<dyn StateCodec>>) -> Self {
        Self { codecs }
    }

    /// The default in-tree codec set. Adding a plugin's codec = one module in
    /// `codecs/` + one line here.
    pub fn standard() -> Self {
        Self::new(vec![
            Arc::new(crate::codecs::vital::VitalCodec),
            Arc::new(crate::codecs::generic::GenericCodec),
        ])
    }

    /// Pick the best codec for this (plugin, blob).
    pub fn select(
        &self,
        plugin: Option<&PluginDescriptor>,
        blob: &[u8],
    ) -> Result<(Arc<dyn StateCodec>, u8), ToolError> {
        self.codecs
            .iter()
            .map(|c| (c.clone(), c.confidence(plugin, blob)))
            .filter(|(_, conf)| *conf > 0)
            .max_by_key(|(_, conf)| *conf)
            .ok_or_else(|| ToolError::internal("no codec matched (generic should always match)"))
    }

    pub fn by_id(&self, id: &str) -> Option<Arc<dyn StateCodec>> {
        self.codecs.iter().find(|c| c.id() == id).cloned()
    }

    pub fn list(&self) -> Vec<CodecInfo> {
        self.codecs.iter().map(|c| c.info()).collect()
    }
}

// ---------------------------------------------------------------------------
// Shared JSON-pointer helpers (RFC 6901)

fn unescape(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

/// Split a pointer into (parent pointer, final token).
fn split_pointer(path: &str) -> Result<(&str, String), ToolError> {
    if !path.starts_with('/') {
        return Err(ToolError::invalid_input(format!(
            "invalid JSON pointer {path:?} — must start with '/' (\"\" addresses the root and cannot be edited)"
        )));
    }
    let idx = path.rfind('/').expect("starts with '/'");
    Ok((&path[..idx], unescape(&path[idx + 1..])))
}

/// Apply edits to a decoded document, in order. `Set` may create a missing
/// final object key (or append with index == len); everything else must exist.
pub fn apply_edits(doc: &mut Value, edits: &[StateEdit]) -> Result<(), ToolError> {
    for edit in edits {
        let (parent_path, token) = split_pointer(&edit.path)?;
        let parent = doc.pointer_mut(parent_path).ok_or_else(|| {
            ToolError::not_found(format!(
                "path {:?}: parent {parent_path:?} does not exist in the decoded document",
                edit.path
            ))
        })?;
        match (&edit.op, parent) {
            (EditOp::Set { value }, Value::Object(map)) => {
                map.insert(token, value.clone());
            }
            (EditOp::Set { value }, Value::Array(items)) => {
                let index = parse_index(&token, &edit.path, items.len(), true)?;
                if index == items.len() {
                    items.push(value.clone());
                } else {
                    items[index] = value.clone();
                }
            }
            (EditOp::Remove, Value::Object(map)) => {
                if map.remove(&token).is_none() {
                    return Err(ToolError::not_found(format!(
                        "path {:?} does not exist — nothing to remove",
                        edit.path
                    )));
                }
            }
            (EditOp::Remove, Value::Array(items)) => {
                let index = parse_index(&token, &edit.path, items.len(), false)?;
                items.remove(index);
            }
            (_, other) => {
                return Err(ToolError::invalid_input(format!(
                    "path {:?}: parent is a {} — only objects and arrays can be edited",
                    edit.path,
                    type_name(other)
                )));
            }
        }
    }
    Ok(())
}

fn parse_index(token: &str, path: &str, len: usize, allow_append: bool) -> Result<usize, ToolError> {
    let index: usize = token
        .parse()
        .map_err(|_| ToolError::invalid_input(format!("path {path:?}: {token:?} is not an array index")))?;
    let out_of_bounds = if allow_append { index > len } else { index >= len };
    if out_of_bounds {
        return Err(ToolError::not_found(format!(
            "path {path:?}: index {index} out of bounds (array has {len} items)"
        )));
    }
    Ok(index)
}

pub fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Prune a document to `depth` levels; deeper structure is summarized so a
/// subtree response can't accidentally be megabytes. Fetch a deeper `path` to
/// drill in.
pub fn prune(value: &Value, depth: u32) -> Value {
    match value {
        Value::Object(map) if depth == 0 => Value::String(format!("{{…}} object, {} keys", map.len())),
        Value::Array(items) if depth == 0 => Value::String(format!("[…] array, {} items", items.len())),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), prune(v, depth - 1))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| prune(v, depth - 1)).collect()),
        Value::String(s) if s.len() > 512 => {
            let mut cut = 256;
            while cut > 0 && !s.is_char_boundary(cut) {
                cut -= 1;
            }
            Value::String(format!("{}… (string, {} chars)", &s[..cut], s.chars().count()))
        }
        leaf => leaf.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(path: &str, value: Value) -> StateEdit {
        StateEdit { path: path.into(), op: EditOp::Set { value } }
    }

    #[test]
    fn set_replaces_and_creates_keys() {
        let mut doc = json!({"settings": {"cutoff": 0.5}});
        apply_edits(
            &mut doc,
            &[set("/settings/cutoff", json!(0.9)), set("/settings/new_key", json!("x"))],
        )
        .unwrap();
        assert_eq!(doc, json!({"settings": {"cutoff": 0.9, "new_key": "x"}}));
    }

    #[test]
    fn array_set_and_append() {
        let mut doc = json!({"items": [1, 2]});
        apply_edits(&mut doc, &[set("/items/0", json!(10)), set("/items/2", json!(3))]).unwrap();
        assert_eq!(doc, json!({"items": [10, 2, 3]}));
        assert!(apply_edits(&mut doc, &[set("/items/9", json!(0))]).is_err());
    }

    #[test]
    fn remove_object_key_and_array_item() {
        let mut doc = json!({"a": 1, "items": [1, 2, 3]});
        apply_edits(
            &mut doc,
            &[
                StateEdit { path: "/a".into(), op: EditOp::Remove },
                StateEdit { path: "/items/1".into(), op: EditOp::Remove },
            ],
        )
        .unwrap();
        assert_eq!(doc, json!({"items": [1, 3]}));
    }

    #[test]
    fn missing_parent_and_bad_pointer_error() {
        let mut doc = json!({});
        assert!(apply_edits(&mut doc, &[set("/no/such/parent", json!(1))]).is_err());
        assert!(apply_edits(&mut doc, &[set("no-slash", json!(1))]).is_err());
    }

    #[test]
    fn escaped_pointer_tokens() {
        let mut doc = json!({"a/b": {"c~d": 1}});
        apply_edits(&mut doc, &[set("/a~1b/c~0d", json!(2))]).unwrap();
        assert_eq!(doc, json!({"a/b": {"c~d": 2}}));
    }

    #[test]
    fn prune_summarizes_below_depth() {
        let doc = json!({"a": {"b": {"c": 1}}, "list": [1, 2, 3]});
        let pruned = prune(&doc, 1);
        assert_eq!(pruned["a"], json!("{…} object, 1 keys"));
        assert_eq!(pruned["list"], json!("[…] array, 3 items"));
    }
}
