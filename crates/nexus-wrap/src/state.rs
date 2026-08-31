//! State serialization and deserialization for nexus-wrap.
//!
//! Preserves the inner plugin's full state blob along with wrapper metadata
//! across DAW project saves and restores.

use std::io::{Read, Write};
use clack_plugin::plugin::PluginError;
use clack_plugin::stream::{InputStream, OutputStream};
use serde::{Deserialize, Serialize};

/// The state envelope saved into the DAW project chunk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WrapStateEnvelope {
    /// State envelope format version.
    pub version: u32,
    /// Inner plugin identifier (e.g. "com.u-he.diva" or "vital").
    pub plugin_id: Option<String>,
    /// Inner plugin format ("clap" or "vst3").
    pub format: Option<String>,
    /// Inner plugin display name.
    pub name: Option<String>,
    /// Raw state blob captured from the inner plugin instance.
    pub inner_state: Vec<u8>,
}

impl WrapStateEnvelope {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn new(
        plugin_id: Option<String>,
        format: Option<String>,
        name: Option<String>,
        inner_state: Vec<u8>,
    ) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            plugin_id,
            format,
            name,
            inner_state,
        }
    }

    /// Serialize this envelope to a CLAP output stream.
    pub fn save(&self, output: &mut OutputStream) -> Result<(), PluginError> {
        let json_bytes = serde_json::to_vec(self).map_err(|e| {
            log::error!("nexus-wrap: failed to serialize state envelope: {e}");
            PluginError::Message("failed to serialize state envelope")
        })?;

        output.write_all(&json_bytes).map_err(|e| {
            log::error!("nexus-wrap: failed to write state to output stream: {e}");
            PluginError::Message("failed to write state to stream")
        })?;

        Ok(())
    }

    /// Deserialize an envelope from a CLAP input stream.
    pub fn load(input: &mut InputStream) -> Result<Self, PluginError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes).map_err(|e| {
            log::error!("nexus-wrap: failed to read state from input stream: {e}");
            PluginError::Message("failed to read state from stream")
        })?;

        if bytes.is_empty() {
            return Ok(Self::new(None, None, None, Vec::new()));
        }

        let envelope: Self = serde_json::from_slice(&bytes).map_err(|e| {
            log::error!("nexus-wrap: failed to deserialize state envelope: {e}");
            PluginError::Message("failed to deserialize state envelope")
        })?;

        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_envelope_round_trip() {
        let envelope = WrapStateEnvelope::new(
            Some("com.u-he.diva".into()),
            Some("clap".into()),
            Some("Diva".into()),
            vec![1, 2, 3, 4, 5, 255],
        );

        let json = serde_json::to_vec(&envelope).unwrap();
        let loaded: WrapStateEnvelope = serde_json::from_slice(&json).unwrap();
        assert_eq!(envelope, loaded);
    }
}
