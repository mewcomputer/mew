//! Tolerant wire codec shared by all non-browser clients.

use mew_protocol::{ClientMessage, ServerMessage};
use tracing::warn;

/// Decode a server frame without terminating a connection when a newer daemon
/// adds a variant this client does not know yet.
pub fn decode_server_message_lenient(
    text: &str,
) -> Result<Option<ServerMessage>, serde_json::Error> {
    match serde_json::from_str::<ServerMessage>(text) {
        Ok(message) => Ok(Some(message)),
        Err(decode_error) => {
            let value: serde_json::Value = serde_json::from_str(text)?;
            let type_tag = value
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            warn!(%type_tag, error = %decode_error, "dropping undecodable ServerMessage frame");
            Ok(None)
        }
    }
}

/// Encode an outgoing protocol command as a WebSocket text payload.
pub fn encode_client_message(message: &ClientMessage) -> Result<String, serde_json::Error> {
    mew_protocol::encode_json(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_messages_decode() {
        let message = decode_server_message_lenient(r#"{"type":"pong","version":"0.2.0"}"#)
            .unwrap()
            .expect("known message");
        assert!(matches!(message, ServerMessage::Pong { version } if version == "0.2.0"));
    }

    #[test]
    fn unknown_and_malformed_messages_are_dropped() {
        assert!(
            decode_server_message_lenient(r#"{"type":"future_variant","value":true}"#)
                .unwrap()
                .is_none()
        );
        assert!(decode_server_message_lenient(r#"{"type":"pong"}"#)
            .unwrap()
            .is_none());
    }

    #[test]
    fn invalid_json_remains_an_error() {
        assert!(decode_server_message_lenient("not json").is_err());
    }
}
