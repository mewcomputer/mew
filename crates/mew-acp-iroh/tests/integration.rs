use anyhow::Result;
use mew_acp::Transport;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

fn make_agent() -> mew_agent::Agent {
    let provider = Arc::new(mew_provider_fake::FakeProvider::new(
        mew_provider_fake::FakeProvider::text_response("hello from iroh"),
    ));
    let dispatcher = Arc::new(mew_hooks::NopDispatcher);
    mew_agent::Agent::new(provider, dispatcher, None, vec![], None)
}

async fn send_line(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    value: &serde_json::Value,
) -> Result<()> {
    let line = serde_json::to_string(value)?;
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

#[tokio::test]
async fn test_pairing_ticket_roundtrip() {
    let ticket = mew_acp_iroh::PairingTicket {
        public_key: "abc123".to_string(),
        relay_urls: vec!["https://relay.example.com".to_string()],
        direct_addresses: vec!["127.0.0.1:12345".to_string()],
    };
    let encoded = ticket.encode().unwrap();
    let decoded = mew_acp_iroh::PairingTicket::decode(&encoded).unwrap();
    assert_eq!(decoded.public_key, "abc123");
    assert_eq!(decoded.relay_urls, vec!["https://relay.example.com"]);
    assert_eq!(decoded.direct_addresses, vec!["127.0.0.1:12345"]);
}

#[tokio::test]
async fn test_pairing_ticket_roundtrip_empty_fields() {
    let ticket = mew_acp_iroh::PairingTicket {
        public_key: String::new(),
        relay_urls: vec![],
        direct_addresses: vec![],
    };
    let encoded = ticket.encode().unwrap();
    let decoded = mew_acp_iroh::PairingTicket::decode(&encoded).unwrap();
    assert!(decoded.public_key.is_empty());
    assert!(decoded.relay_urls.is_empty());
    assert!(decoded.direct_addresses.is_empty());
}

#[tokio::test]
async fn test_pairing_ticket_rejects_garbage() {
    assert!(mew_acp_iroh::PairingTicket::decode("not-valid-base64!!!").is_err());
}

#[tokio::test]
async fn test_pairing_code_format() {
    for _ in 0..20 {
        let code = mew_acp_iroh::generate_pairing_code();
        assert_eq!(code.len(), 7, "code should be XXX-XXX: got {code}");
        assert!(code.contains('-'));
        let parts: Vec<&str> = code.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 3);
        assert_eq!(parts[1].len(), 3);
        assert!(parts[0].parse::<u32>().is_ok());
        assert!(parts[1].parse::<u32>().is_ok());
    }
}

#[tokio::test]
async fn test_iroh_transport_with_duplex() {
    let agent = make_agent();

    let (cr, sw) = tokio::io::duplex(4096);
    let (sr, cw) = tokio::io::duplex(4096);

    tokio::spawn(async move {
        let _ = mew_acp::run_server_on(
            agent,
            mew_acp_iroh::IrohTransport::new(BufReader::new(sr), sw),
        )
        .await;
    });

    let mut reader = BufReader::new(cr);
    let mut writer = BufWriter::new(cw);

    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": {}}
    });
    send_line(&mut writer, &init).await.unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let resp: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(resp["id"], 1);
    assert!(resp["result"].is_object());

    let new_sess = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "session/new",
        "params": {"cwd": "/tmp", "mcpServers": []}
    });
    send_line(&mut writer, &new_sess).await.unwrap();

    let session_id = loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let msg: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        if msg.get("id").and_then(|v| v.as_u64()) == Some(2) {
            break msg["result"]["sessionId"].as_str().unwrap().to_string();
        }
    };

    let prompt = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "session/prompt",
        "params": {
            "sessionId": session_id,
            "prompt": [{"type": "text", "text": "hi"}]
        }
    });
    send_line(&mut writer, &prompt).await.unwrap();

    let got_response = loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let msg: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        if msg.get("id").and_then(|v| v.as_u64()) == Some(3) {
            break true;
        }
    };
    assert!(got_response);
}

#[tokio::test]
async fn test_iroh_transport_split() {
    let (cr, sw) = tokio::io::duplex(256);
    let (sr, cw) = tokio::io::duplex(256);
    let transport = mew_acp_iroh::IrohTransport::new(BufReader::new(sr), sw);
    let (_r, _w) = transport.split();

    let transport2 = mew_acp_iroh::IrohTransport::new(BufReader::new(cr), cw);
    let (_r2, _w2) = transport2.split();
}
