//! Doc-level structural diff between two decoded state documents.
//!
//! This is the investigation loop's core read-out: "what did that knob
//! change?". Byte diffs are useless for compressed states (one knob turn
//! rewrites every compressed byte); comparing the *decoded* documents names
//! the changed paths instead.

use serde_json::Value;

use crate::codec::type_name;
use crate::types::{DiffChange, DiffKind};

/// Stop after this many changes (the response says it was truncated).
const MAX_CHANGES: usize = 500;
/// Values whose serialization exceeds this are summarized, not embedded.
const MAX_VALUE_BYTES: usize = 256;

pub struct DiffOutcome {
    pub changes: Vec<DiffChange>,
    pub truncated: bool,
}

pub fn diff_docs(a: &Value, b: &Value) -> DiffOutcome {
    let mut changes = Vec::new();
    let mut truncated = false;
    walk(a, b, &mut String::new(), &mut changes, &mut truncated);
    DiffOutcome { changes, truncated }
}

fn walk(a: &Value, b: &Value, path: &mut String, out: &mut Vec<DiffChange>, truncated: &mut bool) {
    if *truncated || a == b {
        return;
    }
    match (a, b) {
        (Value::Object(ma), Value::Object(mb)) => {
            for (key, va) in ma {
                let len = path.len();
                push_token(path, key);
                match mb.get(key) {
                    Some(vb) => walk(va, vb, path, out, truncated),
                    None => record(out, truncated, path, DiffKind::Removed, Some(va), None),
                }
                path.truncate(len);
            }
            for (key, vb) in mb {
                if !ma.contains_key(key) {
                    let len = path.len();
                    push_token(path, key);
                    record(out, truncated, path, DiffKind::Added, None, Some(vb));
                    path.truncate(len);
                }
            }
        }
        (Value::Array(ia), Value::Array(ib)) if ia.len() == ib.len() => {
            for (index, (va, vb)) in ia.iter().zip(ib).enumerate() {
                let len = path.len();
                push_token(path, &index.to_string());
                walk(va, vb, path, out, truncated);
                path.truncate(len);
            }
        }
        // Different types, differing scalars, or arrays of different length:
        // one change at this path (element-wise diff of shifted arrays is noise).
        _ => record(out, truncated, path, DiffKind::Changed, Some(a), Some(b)),
    }
}

fn record(
    out: &mut Vec<DiffChange>,
    truncated: &mut bool,
    path: &str,
    kind: DiffKind,
    before: Option<&Value>,
    after: Option<&Value>,
) {
    if out.len() >= MAX_CHANGES {
        *truncated = true;
        return;
    }
    out.push(DiffChange {
        path: path.to_string(),
        kind,
        before: before.map(summarize),
        after: after.map(summarize),
    });
}

/// Embed small values verbatim; describe large ones.
fn summarize(value: &Value) -> Value {
    let size = serde_json::to_string(value).map(|s| s.len()).unwrap_or(usize::MAX);
    if size <= MAX_VALUE_BYTES {
        return value.clone();
    }
    match value {
        Value::Object(map) => Value::String(format!("{{…}} object, {} keys", map.len())),
        Value::Array(items) => Value::String(format!("[…] array, {} items", items.len())),
        Value::String(s) => Value::String(format!("string, {} chars", s.chars().count())),
        other => Value::String(format!("{}, {size} bytes", type_name(other))),
    }
}

fn push_token(path: &mut String, token: &str) {
    path.push('/');
    for c in token.chars() {
        match c {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            c => path.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalar_change_is_named_by_path() {
        let a = json!({"settings": {"cutoff": 0.42, "res": 0.1}});
        let b = json!({"settings": {"cutoff": 0.58, "res": 0.1}});
        let outcome = diff_docs(&a, &b);
        assert_eq!(outcome.changes.len(), 1);
        let change = &outcome.changes[0];
        assert_eq!(change.path, "/settings/cutoff");
        assert_eq!(change.kind, DiffKind::Changed);
        assert_eq!(change.before, Some(json!(0.42)));
        assert_eq!(change.after, Some(json!(0.58)));
    }

    #[test]
    fn added_and_removed_keys() {
        let a = json!({"gone": 1, "same": 2});
        let b = json!({"same": 2, "new": 3});
        let outcome = diff_docs(&a, &b);
        assert_eq!(outcome.changes.len(), 2);
        assert!(outcome.changes.iter().any(|c| c.path == "/gone" && c.kind == DiffKind::Removed));
        assert!(outcome.changes.iter().any(|c| c.path == "/new" && c.kind == DiffKind::Added));
    }

    #[test]
    fn equal_length_arrays_diff_elementwise_different_lengths_as_one() {
        let outcome = diff_docs(&json!([1, 2, 3]), &json!([1, 9, 3]));
        assert_eq!(outcome.changes.len(), 1);
        assert_eq!(outcome.changes[0].path, "/1");

        let outcome = diff_docs(&json!({"l": [1, 2]}), &json!({"l": [1, 2, 3]}));
        assert_eq!(outcome.changes.len(), 1);
        assert_eq!(outcome.changes[0].path, "/l");
    }

    #[test]
    fn pointer_tokens_are_escaped() {
        let outcome = diff_docs(&json!({"a/b": 1}), &json!({"a/b": 2}));
        assert_eq!(outcome.changes[0].path, "/a~1b");
    }

    #[test]
    fn huge_values_are_summarized() {
        let big: Vec<i64> = (0..1000).collect();
        let outcome = diff_docs(&json!({"x": 1}), &json!({"x": big}));
        assert_eq!(outcome.changes[0].after, Some(json!("[…] array, 1000 items")));
    }
}
