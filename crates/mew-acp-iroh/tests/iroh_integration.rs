/// Real iroh endpoint integration test.
///
/// Creates two iroh endpoints on loopback, connects them directly,
/// runs OPAQUE registration + login with AEAD wrapping, then a full
/// ACP conversation (init → session → prompt → response).
///
/// Opt-in: set MEW_IROH_INTEGRATION=1 to run.
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

use iroh::endpoint::presets;
use mew_acp::Transport;
use mew_acp_iroh::{aead, pairing, IrohTransport};

const ALPN: &[u8] = b"mew/acp/1";

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
async fn test_iroh_acp_full_flow() {
    if std::env::var("MEW_IROH_INTEGRATION").is_err() {
        eprintln!("skipping (set MEW_IROH_INTEGRATION=1 to run)");
        return;
    }

    let agent = make_agent();
    let password = b"123-456";

    // Bind server endpoint first so client can use its direct addresses.
    let server_ep = iroh::Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .expect("server bind");

    let server_addr = server_ep.addr();
    let server_node_id = server_addr.id;

    let server_handle = tokio::spawn(async move {
        let incoming = server_ep.accept().await.expect("server accept");
        let conn = incoming.await.expect("server connection");

        let (mut send, mut recv) = conn.accept_bi().await.expect("server accept_bi");

        let mut ps = pairing::PairingServer::new();
        ps.register(&mut recv, &mut send).await.expect("register");
        let sk = ps.login(&mut recv, &mut send).await.expect("login");
        let ak = aead::key_from_session(&sk);

        let reader = aead::AeadReader::new(recv, &ak);
        let writer = aead::AeadWriter::new(send, &ak);
        let transport = IrohTransport::new(reader, writer);

        mew_acp::run_server_on(agent, transport)
            .await
            .expect("acp server");
    });

    // Bind client endpoint.
    let client_ep = iroh::Endpoint::builder(presets::N0)
        .bind()
        .await
        .expect("client bind");

    // Create direct address from server.
    let mut addr = iroh::EndpointAddr::new(server_node_id);
    for direct in server_addr.ip_addrs() {
        addr = addr.with_ip_addr(*direct);
    }

    let conn = client_ep.connect(addr, ALPN).await.expect("client connect");

    let (mut send, mut recv) = conn.open_bi().await.expect("client open_bi");

    // OPAQUE pairing + AEAD wrapping.
    pairing::client_register(password, &mut recv, &mut send)
        .await
        .expect("client register");
    let sk = pairing::client_login(password, &mut recv, &mut send)
        .await
        .expect("client login");
    let ak = aead::key_from_session(&sk);

    let reader = aead::AeadReader::new(recv, &ak);
    let writer = aead::AeadWriter::new(send, &ak);
    let transport = IrohTransport::new(reader, writer);

    let (_r, w) = transport.split();
    let mut reader = BufReader::new(_r);
    let mut writer = BufWriter::new(w);

    // ACP init
    let init = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": {}}
    });
    send_line(&mut writer, &init).await.expect("send init");

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .expect("read init response");
    let resp: serde_json::Value = serde_json::from_str(line.trim()).expect("parse init resp");
    assert_eq!(resp["id"], 1);

    // ACP session/new
    let new_sess = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "session/new",
        "params": {"cwd": "/tmp", "mcpServers": []}
    });
    send_line(&mut writer, &new_sess)
        .await
        .expect("send session/new");

    let mut session_id = String::new();
    loop {
        line.clear();
        reader
            .read_line(&mut line)
            .await
            .expect("read session response");
        let msg: serde_json::Value = serde_json::from_str(line.trim()).expect("parse");
        if msg.get("id").and_then(|v| v.as_u64()) == Some(2) {
            session_id = msg["result"]["sessionId"]
                .as_str()
                .expect("sessionId")
                .to_string();
            break;
        }
    }
    assert!(!session_id.is_empty());

    // ACP session/prompt
    let prompt = serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "session/prompt",
        "params": {
            "sessionId": session_id,
            "prompt": [{"type": "text", "text": "hello"}]
        }
    });
    send_line(&mut writer, &prompt).await.expect("send prompt");

    let mut got_response = false;
    loop {
        line.clear();
        reader
            .read_line(&mut line)
            .await
            .expect("read prompt response");
        let msg: serde_json::Value = serde_json::from_str(line.trim()).expect("parse");
        if msg.get("id").and_then(|v| v.as_u64()) == Some(3) {
            got_response = true;
            break;
        }
    }
    assert!(got_response, "should get response to session/prompt");

    server_handle.abort();
}
