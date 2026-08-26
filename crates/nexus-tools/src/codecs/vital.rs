//! Vital (and its Serum-alike forks): state is pure JSON.
//!
//! The decoded document IS the plugin's JSON state — edit paths address it
//! directly (e.g. `/settings/osc_1_level`). Encode is plain serialization;
//! Vital accepts any valid JSON re-encoding of its state (verified through the
//! plugin by `edit_state`).

use nexus_host::host::PluginDescriptor;
use serde_json::Value;

use crate::codec::StateCodec;
use crate::error::ToolError;

pub struct VitalCodec;

impl StateCodec for VitalCodec {
    fn id(&self) -> &'static str {
        "vital-json"
    }

    fn description(&self) -> &'static str {
        "Vital's JSON state, decoded 1:1 (paths address the state directly); encode-capable"
    }

    fn confidence(&self, plugin: Option<&PluginDescriptor>, blob: &[u8]) -> u8 {
        let named_vital = plugin.is_some_and(|d| {
            let hay = format!(
                "{} {}",
                d.id.to_lowercase(),
                d.name.as_deref().unwrap_or("").to_lowercase()
            );
            hay.contains("vital")
        });
        if named_vital && serde_json::from_slice::<Value>(blob).is_ok() { 220 } else { 0 }
    }

    fn decode(&self, _plugin: Option<&PluginDescriptor>, blob: &[u8]) -> Result<Value, ToolError> {
        serde_json::from_slice(blob)
            .map_err(|e| ToolError::internal(format!("vital-json: state is not valid JSON: {e}")))
    }

    fn encode(&self, _plugin: Option<&PluginDescriptor>, doc: &Value) -> Option<Result<Vec<u8>, ToolError>> {
        Some(serde_json::to_vec(doc).map_err(|e| ToolError::internal(format!("vital-json encode: {e}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vital_descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: "audio.vital.synth".into(),
            name: Some("Vital".into()),
            vendor: Some("Vital Audio".into()),
            version: None,
        }
    }

    #[test]
    fn matches_only_vital_with_json_state() {
        let codec = VitalCodec;
        let json = br#"{"settings": {}}"#;
        assert!(codec.confidence(Some(&vital_descriptor()), json) > 1);
        assert_eq!(codec.confidence(Some(&vital_descriptor()), b"\x00binary"), 0);
        let other = PluginDescriptor { id: "com.serum".into(), name: Some("Serum 2".into()), vendor: None, version: None };
        assert_eq!(codec.confidence(Some(&other), json), 0);
        assert_eq!(codec.confidence(None, json), 0);
    }

    #[test]
    fn decode_encode_round_trips() {
        let codec = VitalCodec;
        let blob = br#"{"settings": {"osc_1_level": 0.7, "wavetables": [1, 2]}}"#;
        let doc = codec.decode(None, blob).unwrap();
        let encoded = codec.encode(None, &doc).unwrap().unwrap();
        let doc2 = codec.decode(None, &encoded).unwrap();
        assert_eq!(doc, doc2);
    }
}
