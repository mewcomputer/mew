//! Iroh transport for ACP.
//!
//! Implements the ACP `Transport` trait over iroh QUIC connections,
//! with OPAQUE-based pairing and AEAD-encrypted framing.

use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, PublicKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncWrite};
use tracing::info;

use mew_acp::Transport;

pub mod aead;
pub mod pairing;

const ALPN: &[u8] = b"mew/acp/1";

// ---------------------------------------------------------------------------
// Transport wrapper
// ---------------------------------------------------------------------------

pub struct IrohTransport<R: AsyncBufRead + Unpin + Send, W: AsyncWrite + Unpin + Send> {
    reader: R,
    writer: W,
}

impl<R: AsyncBufRead + Unpin + Send, W: AsyncWrite + Unpin + Send> IrohTransport<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R: AsyncBufRead + Unpin + Send + 'static, W: AsyncWrite + Unpin + Send + 'static> Transport
    for IrohTransport<R, W>
{
    type Reader = R;
    type Writer = W;

    fn split(self) -> (Self::Reader, Self::Writer) {
        (self.reader, self.writer)
    }
}

// ---------------------------------------------------------------------------
// Ticket serialization
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct PairingTicket {
    pub public_key: String,
    pub relay_urls: Vec<String>,
    pub direct_addresses: Vec<String>,
}

impl PairingTicket {
    pub fn encode(&self) -> Result<String> {
        let json = serde_json::to_string(self)?;
        Ok(URL_SAFE_NO_PAD.encode(json))
    }

    pub fn decode(s: &str) -> Result<Self> {
        let json = URL_SAFE_NO_PAD
            .decode(s)
            .context("invalid ticket encoding")?;
        serde_json::from_slice(&json).context("invalid ticket format")
    }
}

// ---------------------------------------------------------------------------
// Pairing code
// ---------------------------------------------------------------------------

pub fn generate_pairing_code() -> String {
    let mut rng = OsRng;
    let code: u32 = rand::Rng::gen_range(&mut rng, 0..1_000_000);
    format!("{:03}-{:03}", code / 1000, code % 1000)
}

// ---------------------------------------------------------------------------
// Server: listen for iroh connections
// ---------------------------------------------------------------------------

pub async fn listen(agent: mew_agent::Agent) -> Result<()> {
    let endpoint = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("bind iroh endpoint")?;

    let addr = endpoint.addr();
    let public_key = addr.id;

    let mut relay_urls = Vec::new();
    for url in addr.relay_urls() {
        relay_urls.push(url.to_string());
    }
    let mut direct_addrs = Vec::new();
    for socket in addr.ip_addrs() {
        direct_addrs.push(socket.to_string());
    }

    let ticket = PairingTicket {
        public_key: public_key.to_string(),
        relay_urls,
        direct_addresses: direct_addrs,
    };

    let ticket_str = ticket.encode()?;
    let code = generate_pairing_code();

    println!("ticket: {ticket_str}");
    println!("pairing code: {code}");
    println!("waiting for client...");

    info!(node = %public_key, "iroh server waiting for connection");

    let incoming = endpoint.accept().await.context("no incoming connection")?;
    let conn = incoming.await.context("accept connection")?;

    info!("client connected, starting OPAQUE pairing");

    let (mut send, mut recv) = conn
        .accept_bi()
        .await
        .context("accept bidirectional stream")?;

    let mut pairing_server = pairing::PairingServer::new();
    pairing_server.register(&mut recv, &mut send).await?;
    let session_key = pairing_server.login(&mut recv, &mut send).await?;
    let aead_key = aead::key_from_session(&session_key);

    info!("OPAQUE pairing complete, starting encrypted ACP");

    let reader = aead::AeadReader::new(recv, &aead_key);
    let writer = aead::AeadWriter::new(send, &aead_key);

    let transport = IrohTransport { reader, writer };

    mew_acp::run_server_on(agent, transport).await
}

// ---------------------------------------------------------------------------
// Client: connect to an iroh agent server
// ---------------------------------------------------------------------------

pub async fn connect(ticket_str: &str, pairing_code: &str) -> Result<(impl Transport, Endpoint)> {
    let ticket = PairingTicket::decode(ticket_str)?;

    let public_key: PublicKey = ticket
        .public_key
        .parse()
        .context("parse public key from ticket")?;

    let endpoint = Endpoint::builder(presets::N0)
        .bind()
        .await
        .context("bind iroh endpoint")?;

    let mut addr = EndpointAddr::new(public_key);
    for relay in &ticket.relay_urls {
        let url: iroh::RelayUrl = relay.parse().context("parse relay url")?;
        addr = addr.with_relay_url(url);
    }

    let conn = endpoint
        .connect(addr, ALPN)
        .await
        .context("connect to iroh node")?;

    info!("connected to agent, starting OPAQUE pairing");

    let (mut send, mut recv) = conn.open_bi().await.context("open bidirectional stream")?;

    let password = pairing_code.as_bytes();
    pairing::client_register(password, &mut recv, &mut send).await?;
    let session_key = pairing::client_login(password, &mut recv, &mut send).await?;
    let aead_key = aead::key_from_session(&session_key);

    info!("OPAQUE pairing complete, starting encrypted ACP");

    let reader = aead::AeadReader::new(recv, &aead_key);
    let writer = aead::AeadWriter::new(send, &aead_key);

    let transport = IrohTransport { reader, writer };

    Ok((transport, endpoint))
}
