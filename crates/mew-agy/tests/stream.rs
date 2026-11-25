use mew_agy::stream::{AcpFrame, AcpStream, RpcError};
use serde_json::json;

#[test]
fn streams_fragmented_text_and_preserves_unicode() {
    let mut stream = AcpStream::new(4096);
    let input = "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"café\"}}}}\n".as_bytes();
    let split = input.iter().position(|b| *b == 0xc3).unwrap() + 1;
    assert!(stream.push(&input[..split]).unwrap().is_empty());
    assert_eq!(
        stream.push(&input[split..]).unwrap(),
        vec![AcpFrame::Notification {
            method: "session/update".into(),
            params: json!({
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "café" }
                }
            }),
        }]
    );
}

#[test]
fn parses_responses_errors_and_server_requests() {
    let mut stream = AcpStream::new(4096);
    let frames = stream
        .push(
            br#"{"jsonrpc":"2.0","id":3,"result":{"sessionId":"s1"}}
{"jsonrpc":"2.0","id":"perm-1","method":"session/request_permission","params":{"options":[]}}
{"jsonrpc":"2.0","id":5,"error":{"code":-32000,"message":"Authentication required"}}
"#,
        )
        .unwrap();
    assert_eq!(
        frames,
        vec![
            AcpFrame::Response {
                id: 3,
                result: Some(json!({"sessionId": "s1"})),
                error: None,
            },
            AcpFrame::Request {
                id: json!("perm-1"),
                method: "session/request_permission".into(),
                params: json!({"options": []}),
            },
            AcpFrame::Response {
                id: 5,
                result: None,
                error: Some(RpcError {
                    code: -32000,
                    message: "Authentication required".into(),
                }),
            },
        ]
    );
}

#[test]
fn rejects_oversized_and_non_jsonrpc_frames() {
    let mut oversized = AcpStream::new(8);
    assert!(oversized.push(b"123456789").is_err());
    let mut wrong_version = AcpStream::new(4096);
    assert!(wrong_version
        .push(b"{\"jsonrpc\":\"1.0\",\"id\":1,\"result\":{}}\n")
        .is_err());
}
