//! Lenient codec for decoding ServerMessages.
//!
//! Per the iOS spec: decode to `serde_json::Value` first, then to the typed
//! enum. If full decode fails, log the `type` tag and drop the frame. This
//! prevents a newer daemon adding a ServerMessage variant from killing an
//! older phone's connection.

use mew_protocol::ServerMessage;
use tracing::warn;

/// Decode a JSON string into a `ServerMessage`, tolerating unknown variants.
///
/// Returns `Ok(None)` if the frame is valid JSON but contains an unknown
/// `type` tag (or a known type with unexpected fields). Returns `Err` only
/// if the frame is not valid JSON at all.
pub fn decode_server_message_lenient(text: &str) -> Result<Option<ServerMessage>, serde_json::Error> {
    // First try direct decode — fast path for known messages.
    match serde_json::from_str::<ServerMessage>(text) {
        Ok(msg) => Ok(Some(msg)),
        Err(e) => {
            // Fall back to lenient: parse as Value, extract type tag.
            let value: serde_json::Value = serde_json::from_str(text)?;
            let type_tag = value
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");

            warn!(
                type_tag = %type_tag,
                error = %e,
                "dropping undecodable ServerMessage frame"
            );
            Ok(None)
        }
    }
}

/// Encode a ClientMessage to a JSON string.
pub fn encode_client_message(msg: &mew_protocol::ClientMessage) -> Result<String, serde_json::Error> {
    serde_json::to_string(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_message_decodes() {
        let json = r#"{"type":"pong","version":"0.2.0"}"#;
        let result = decode_server_message_lenient(json).unwrap();
        assert!(result.is_some());
        match result.unwrap() {
            ServerMessage::Pong { version } => assert_eq!(version, "0.2.0"),
            _ => panic!("expected Pong"),
        }
    }

    #[test]
    fn test_unknown_type_is_dropped() {
        let json = r#"{"type":"some_future_variant","data":"stuff"}"#;
        let result = decode_server_message_lenient(json).unwrap();
        assert!(result.is_none(), "unknown variant should be dropped");
    }

    #[test]
    fn test_invalid_json_returns_error() {
        let json = r#"not json at all"#;
        assert!(decode_server_message_lenient(json).is_err());
    }

    #[test]
    fn test_known_type_bad_fields_is_dropped() {
        // Pong expects "version" field; missing it should be dropped, not crash.
        let json = r#"{"type":"pong"}"#;
        let result = decode_server_message_lenient(json).unwrap();
        assert!(result.is_none(), "malformed known message should be dropped");
    }
}
