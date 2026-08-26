//! The generic fallback codec: best-effort recursive sniffing of an unknown
//! state blob. Decode-only, confidence 1 — any authored codec outranks it.
//!
//! The output is a region tree. Each region is an object with a `kind` plus
//! kind-specific fields; containers recurse into `content`/`children`. Binary
//! leaves carry entropy, printable ratio, extracted strings, and hex windows —
//! enough for a human (or an LLM) to start locating meaningful areas.
//!
//! Known caveat (by design): region addressing is positional and not stable
//! across plugin versions. Good enough for interactive investigation.

use std::io::Read;

use nexus_host::host::PluginDescriptor;
use serde_json::{Value, json};

use crate::codec::StateCodec;
use crate::error::ToolError;

/// Stop recursing below this depth (containers nested deeper stay opaque).
const MAX_DEPTH: u32 = 6;
/// Refuse to expand a compressed region beyond this many bytes.
const MAX_DECOMPRESSED: u64 = 32 * 1024 * 1024;
/// Caps that keep responses LLM-sized.
const MAX_STRINGS: usize = 24;
const MAX_RIFF_CHUNKS: usize = 16;
const HEAD_HEX_BYTES: usize = 64;
const TAIL_HEX_BYTES: usize = 32;

pub struct GenericCodec;

impl StateCodec for GenericCodec {
    fn id(&self) -> &'static str {
        "generic"
    }

    fn description(&self) -> &'static str {
        "best-effort sniffing of unknown blobs (gzip/zlib, RIFF, VST3 preset framing, JSON, XML, binary analysis); decode-only"
    }

    fn confidence(&self, _plugin: Option<&PluginDescriptor>, _blob: &[u8]) -> u8 {
        1
    }

    fn decode(&self, _plugin: Option<&PluginDescriptor>, blob: &[u8]) -> Result<Value, ToolError> {
        Ok(sniff(blob, MAX_DEPTH))
    }

    fn encode(&self, _plugin: Option<&PluginDescriptor>, _doc: &Value) -> Option<Result<Vec<u8>, ToolError>> {
        None
    }
}

/// Sniff one region. Never fails — the worst case is a binary-leaf description.
pub fn sniff(data: &[u8], depth: u32) -> Value {
    if depth == 0 {
        return binary_leaf(data, false);
    }
    if data.is_empty() {
        return json!({ "kind": "empty", "size_bytes": 0 });
    }
    if let Some(region) = try_gzip(data, depth) {
        return region;
    }
    if let Some(region) = try_zlib(data, depth) {
        return region;
    }
    if let Some(region) = try_vst3_preset(data, depth) {
        return region;
    }
    if let Some(region) = try_riff(data, depth) {
        return region;
    }
    if let Some(region) = try_json(data) {
        return region;
    }
    if let Some(region) = try_xml(data) {
        return region;
    }
    binary_leaf(data, true)
}

fn try_gzip(data: &[u8], depth: u32) -> Option<Value> {
    if data.len() < 2 || data[0] != 0x1f || data[1] != 0x8b {
        return None;
    }
    decompress(flate2::read::GzDecoder::new(data)).map(|inner| {
        json!({
            "kind": "gzip",
            "size_bytes": data.len(),
            "decoded_size_bytes": inner.len(),
            "content": sniff(&inner, depth - 1),
        })
    })
}

fn try_zlib(data: &[u8], depth: u32) -> Option<Value> {
    if data.len() < 2 || data[0] != 0x78 || !matches!(data[1], 0x01 | 0x5e | 0x9c | 0xda) {
        return None;
    }
    decompress(flate2::read::ZlibDecoder::new(data)).map(|inner| {
        json!({
            "kind": "zlib",
            "size_bytes": data.len(),
            "decoded_size_bytes": inner.len(),
            "content": sniff(&inner, depth - 1),
        })
    })
}

fn decompress(mut reader: impl Read) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return Some(out),
            Ok(n) => {
                if out.len() as u64 + n as u64 > MAX_DECOMPRESSED {
                    return None;
                }
                out.extend_from_slice(&chunk[..n]);
            }
            Err(_) => return None,
        }
    }
}

/// Steinberg `.vstpreset` framing: "VST3" magic, version, class id, and a
/// chunk list with `Comp`/`Cont`/… entries. `IComponent::getState` blobs are
/// usually raw, but exported presets show up in corpora.
fn try_vst3_preset(data: &[u8], depth: u32) -> Option<Value> {
    if data.len() < 48 || &data[0..4] != b"VST3" {
        return None;
    }
    let class_id = String::from_utf8_lossy(&data[8..40]).to_string();
    let list_offset = u64::from_le_bytes(data[40..48].try_into().ok()?) as usize;
    if list_offset + 12 > data.len() || &data[list_offset..list_offset + 4] != b"List" {
        return Some(json!({
            "kind": "vst3-preset",
            "size_bytes": data.len(),
            "class_id": class_id,
            "note": "chunk list offset invalid — parsed header only",
        }));
    }
    let count = u32::from_le_bytes(data[list_offset + 4..list_offset + 8].try_into().ok()?) as usize;
    let mut entries = Vec::new();
    let mut cursor = list_offset + 8;
    for _ in 0..count.min(16) {
        if cursor + 20 > data.len() {
            break;
        }
        let id = String::from_utf8_lossy(&data[cursor..cursor + 4]).to_string();
        let offset = u64::from_le_bytes(data[cursor + 4..cursor + 12].try_into().ok()?) as usize;
        let size = u64::from_le_bytes(data[cursor + 12..cursor + 20].try_into().ok()?) as usize;
        let content = if offset <= data.len() && offset + size <= data.len() {
            sniff(&data[offset..offset + size], depth - 1)
        } else {
            json!({ "kind": "invalid", "note": "entry range out of bounds" })
        };
        entries.push(json!({ "id": id, "offset": offset, "size_bytes": size, "content": content }));
        cursor += 20;
    }
    Some(json!({
        "kind": "vst3-preset",
        "size_bytes": data.len(),
        "class_id": class_id,
        "entries": entries,
    }))
}

fn try_riff(data: &[u8], depth: u32) -> Option<Value> {
    if data.len() < 12 || &data[0..4] != b"RIFF" {
        return None;
    }
    let form = String::from_utf8_lossy(&data[8..12]).to_string();
    let mut chunks = Vec::new();
    let mut cursor = 12;
    while cursor + 8 <= data.len() && chunks.len() < MAX_RIFF_CHUNKS {
        let id = String::from_utf8_lossy(&data[cursor..cursor + 4]).to_string();
        let size = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().ok()?) as usize;
        let start = cursor + 8;
        let end = (start + size).min(data.len());
        chunks.push(json!({
            "id": id,
            "offset": start,
            "size_bytes": size,
            "content": sniff(&data[start..end], depth - 1),
        }));
        cursor = start + size + (size & 1); // chunks are word-aligned
    }
    Some(json!({
        "kind": "riff",
        "size_bytes": data.len(),
        "form": form,
        "chunks": chunks,
    }))
}

fn try_json(data: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(data).ok()?;
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    Some(json!({
        "kind": "json",
        "size_bytes": data.len(),
        "value": value,
    }))
}

fn try_xml(data: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(data).ok()?;
    let trimmed = text.trim_start();
    if !trimmed.starts_with("<?xml") && !(trimmed.starts_with('<') && looks_tagish(trimmed)) {
        return None;
    }
    let mut tags: Vec<(String, usize)> = Vec::new();
    let mut root_tag = None;
    for raw in trimmed.split('<').skip(1) {
        let name: String = raw
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
            .collect();
        if name.is_empty() || raw.starts_with('?') || raw.starts_with('!') || raw.starts_with('/') {
            continue;
        }
        if root_tag.is_none() {
            root_tag = Some(name.clone());
        }
        if let Some((_, count)) = tags.iter_mut().find(|(t, _)| *t == name) {
            *count += 1;
        } else if tags.len() < 40 {
            tags.push((name, 1));
        }
    }
    root_tag.as_ref()?;
    let tag_counts: serde_json::Map<String, Value> =
        tags.into_iter().map(|(t, c)| (t, json!(c))).collect();
    Some(json!({
        "kind": "xml",
        "size_bytes": data.len(),
        "root_tag": root_tag,
        "tag_counts": tag_counts,
        "text_preview": preview(trimmed, 500),
    }))
}

fn looks_tagish(text: &str) -> bool {
    text.chars().nth(1).is_some_and(|c| c.is_ascii_alphabetic())
}

fn binary_leaf(data: &[u8], scan_embedded: bool) -> Value {
    let mut region = json!({
        "kind": "binary",
        "size_bytes": data.len(),
        "entropy": (entropy(data) * 100.0).round() / 100.0,
        "printable_ratio": (printable_ratio(data) * 100.0).round() / 100.0,
        "strings": extract_strings(data),
        "head_hex": hex(&data[..data.len().min(HEAD_HEX_BYTES)]),
        "tail_hex": hex(&data[data.len().saturating_sub(TAIL_HEX_BYTES)..]),
    });
    if scan_embedded {
        let embedded = scan_for_embedded(data);
        if !embedded.is_empty() {
            region["embedded"] = Value::Array(embedded);
        }
    }
    region
}

/// Look for recognizable regions at interior offsets of an otherwise binary
/// blob: gzip/zlib magics, XML declarations, and JSON objects.
fn scan_for_embedded(data: &[u8]) -> Vec<Value> {
    let mut found = Vec::new();
    // Compressed streams (first few hits only).
    for offset in 1..data.len().saturating_sub(2) {
        if found.len() >= 4 {
            break;
        }
        let window = &data[offset..];
        let is_gzip = window[0] == 0x1f && window[1] == 0x8b;
        let is_zlib = window[0] == 0x78 && matches!(window[1], 0x01 | 0x5e | 0x9c | 0xda);
        if is_gzip || is_zlib {
            let inner = if is_gzip {
                decompress(flate2::read::GzDecoder::new(window))
            } else {
                decompress(flate2::read::ZlibDecoder::new(window))
            };
            if let Some(inner) = inner.filter(|i| i.len() > 16) {
                found.push(json!({
                    "offset": offset,
                    "kind": if is_gzip { "gzip" } else { "zlib" },
                    "decoded_size_bytes": inner.len(),
                    "content": sniff(&inner, 2),
                }));
            }
        }
    }
    // Embedded XML.
    if let Some(pos) = find(data, b"<?xml") {
        if pos > 0 {
            if let Some(region) = try_xml(&data[pos..]) {
                found.push(json!({ "offset": pos, "kind": "xml", "content": region }));
            }
        }
    }
    // Embedded JSON: try the first few '{' positions that are followed by '"'.
    let mut attempts = 0;
    for pos in 1..data.len().saturating_sub(1) {
        if attempts >= 4 || found.len() >= 8 {
            break;
        }
        if data[pos] == b'{' && data[pos + 1..].iter().take(3).any(|&b| b == b'"') {
            attempts += 1;
            if let Some(value) = parse_json_prefix(&data[pos..]) {
                found.push(json!({ "offset": pos, "kind": "json", "content": value }));
            }
        }
    }
    found
}

/// Parse one JSON value from the start of `data`, tolerating trailing bytes.
fn parse_json_prefix(data: &[u8]) -> Option<Value> {
    let mut stream = serde_json::Deserializer::from_slice(data).into_iter::<Value>();
    match stream.next() {
        Some(Ok(value)) if value.is_object() => Some(value),
        _ => None,
    }
}

fn entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in data {
        counts[b as usize] += 1;
    }
    let len = data.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

fn printable_ratio(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let printable = data.iter().filter(|&&b| (0x20..0x7f).contains(&b) || b == b'\n' || b == b'\t').count();
    printable as f64 / data.len() as f64
}

/// Longest printable runs (≥ 4 chars), deduped, longest first.
fn extract_strings(data: &[u8]) -> Vec<String> {
    let mut strings: Vec<String> = Vec::new();
    let mut current = Vec::new();
    let flush = |current: &mut Vec<u8>, strings: &mut Vec<String>| {
        if current.len() >= 4 {
            let s = String::from_utf8_lossy(current).into_owned();
            if !strings.contains(&s) {
                strings.push(s);
            }
        }
        current.clear();
    };
    for &b in data {
        if (0x20..0x7f).contains(&b) {
            current.push(b);
        } else {
            flush(&mut current, &mut strings);
        }
    }
    flush(&mut current, &mut strings);
    strings.sort_by_key(|s| std::cmp::Reverse(s.len()));
    strings.truncate(MAX_STRINGS);
    strings
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn preview(text: &str, max: usize) -> String {
    let mut cut = max.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text[..cut].to_string()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn whole_blob_json_is_parsed() {
        let region = sniff(br#"{"settings": {"cutoff": 0.5}}"#, 4);
        assert_eq!(region["kind"], "json");
        assert_eq!(region["value"]["settings"]["cutoff"], 0.5);
    }

    #[test]
    fn gzip_wrapped_json_recurses() {
        let blob = gzip(br#"{"a": 1}"#);
        let region = sniff(&blob, 4);
        assert_eq!(region["kind"], "gzip");
        assert_eq!(region["content"]["kind"], "json");
        assert_eq!(region["content"]["value"]["a"], 1);
    }

    #[test]
    fn xml_reports_root_and_tag_histogram() {
        let region = sniff(b"<?xml version=\"1.0\"?><Preset><Param id=\"a\"/><Param id=\"b\"/></Preset>", 4);
        assert_eq!(region["kind"], "xml");
        assert_eq!(region["root_tag"], "Preset");
        assert_eq!(region["tag_counts"]["Param"], 2);
    }

    #[test]
    fn riff_lists_chunks() {
        let mut blob = Vec::new();
        blob.extend_from_slice(b"RIFF");
        blob.extend_from_slice(&20u32.to_le_bytes());
        blob.extend_from_slice(b"TEST");
        blob.extend_from_slice(b"chk1");
        blob.extend_from_slice(&4u32.to_le_bytes());
        blob.extend_from_slice(b"data");
        let region = sniff(&blob, 4);
        assert_eq!(region["kind"], "riff");
        assert_eq!(region["form"], "TEST");
        assert_eq!(region["chunks"][0]["id"], "chk1");
    }

    #[test]
    fn noise_is_a_binary_leaf_with_high_entropy() {
        // Deterministic pseudo-noise (xorshift).
        let mut x = 0x12345678u32;
        let noise: Vec<u8> = (0..4096)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x & 0xff) as u8
            })
            .collect();
        let region = sniff(&noise, 4);
        assert_eq!(region["kind"], "binary");
        assert!(region["entropy"].as_f64().unwrap() > 7.0);
    }

    #[test]
    fn embedded_json_at_offset_is_found() {
        let mut blob = vec![0u8, 1, 2, 3, 0xff, 0xfe];
        blob.extend_from_slice(br#"{"embedded": true}"#);
        blob.extend_from_slice(&[0, 0, 0]);
        let region = sniff(&blob, 4);
        assert_eq!(region["kind"], "binary");
        let embedded = region["embedded"].as_array().expect("embedded regions");
        assert!(embedded.iter().any(|e| e["kind"] == "json" && e["content"]["embedded"] == true));
    }

    #[test]
    fn strings_are_extracted_from_binary() {
        let mut blob = vec![0u8; 16];
        blob.extend_from_slice(b"FilterCutoff");
        blob.extend_from_slice(&[0, 1, 2]);
        let region = sniff(&blob, 4);
        let strings = region["strings"].as_array().unwrap();
        assert!(strings.iter().any(|s| s == "FilterCutoff"));
    }
}
