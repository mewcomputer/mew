//! OPAQUE-based pairing for iroh ACP connections.
//!
//! Two flows:
//!   registration — client pairs with server using a pairing code (one-time setup)
//!   login — client authenticates with the pairing code to establish a session key
//!
//! Both flows produce a shared `session_key` used for AEAD encryption of ACP frames.

use anyhow::{Context, Result};
use opaque_ke::{
    CipherSuite, ClientLogin, ClientLoginFinishParameters, ClientRegistration,
    ClientRegistrationFinishParameters, CredentialFinalization, CredentialRequest,
    CredentialResponse, RegistrationRequest, RegistrationResponse, RegistrationUpload,
    Ristretto255, ServerLogin, ServerLoginParameters, ServerRegistration, ServerSetup, TripleDh,
};
use rand::rngs::OsRng;
use sha2::Sha512;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::info;

const CREDENTIAL_ID: &[u8] = b"mew-iroh-pairing";

// ---------------------------------------------------------------------------
// Cipher suite configuration
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct MewCipherSuite;

impl CipherSuite for MewCipherSuite {
    type OprfCs = Ristretto255;
    type KeyExchange = TripleDh<Ristretto255, Sha512>;
    type Ksf = opaque_ke::ksf::Identity;
}

// ---------------------------------------------------------------------------
// Wire messages (length-prefixed binary frames)
// ---------------------------------------------------------------------------

async fn send_frame(writer: &mut (impl tokio::io::AsyncWrite + Unpin), data: &[u8]) -> Result<()> {
    let len = (data.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

async fn recv_frame(reader: &mut (impl tokio::io::AsyncRead + Unpin)) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 1_000_000 {
        anyhow::bail!("frame too large: {len} bytes");
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

pub struct PairingServer {
    server_setup: ServerSetup<MewCipherSuite>,
    registration: Option<(Vec<u8>, ServerRegistration<MewCipherSuite>)>,
}

impl Default for PairingServer {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingServer {
    pub fn new() -> Self {
        let mut rng = OsRng;
        Self {
            server_setup: ServerSetup::new(&mut rng),
            registration: None,
        }
    }

    pub async fn register(
        &mut self,
        reader: &mut (impl tokio::io::AsyncRead + Unpin),
        writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    ) -> Result<()> {
        let req_bytes = recv_frame(reader).await?;
        let req = RegistrationRequest::deserialize(&req_bytes)
            .context("deserialize registration request")?;

        let server_start = ServerRegistration::start(&self.server_setup, req, CREDENTIAL_ID)
            .context("server registration start")?;
        let resp_bytes = server_start.message.serialize();
        send_frame(writer, &resp_bytes).await?;

        let upload_bytes = recv_frame(reader).await?;
        let upload = RegistrationUpload::deserialize(&upload_bytes)
            .context("deserialize registration upload")?;
        let registration = ServerRegistration::finish(upload);

        self.registration = Some((CREDENTIAL_ID.to_vec(), registration));
        info!("client registered via OPAQUE");

        Ok(())
    }

    pub async fn login(
        &self,
        reader: &mut (impl tokio::io::AsyncRead + Unpin),
        writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    ) -> Result<Vec<u8>> {
        let mut rng = OsRng;

        let req_bytes = recv_frame(reader).await?;
        let req =
            CredentialRequest::deserialize(&req_bytes).context("deserialize credential request")?;

        let (cred_id, registration) = self
            .registration
            .as_ref()
            .context("no registered client — run registration first")?;

        let server_start = ServerLogin::start(
            &mut rng,
            &self.server_setup,
            Some(registration.clone()),
            req,
            cred_id,
            ServerLoginParameters::default(),
        )
        .context("server login start")?;

        let resp_bytes = server_start.message.serialize();
        send_frame(writer, &resp_bytes).await?;

        let fin_bytes = recv_frame(reader).await?;
        let fin = CredentialFinalization::deserialize(&fin_bytes)
            .context("deserialize credential finalization")?;

        let server_finish = server_start
            .state
            .finish(fin, ServerLoginParameters::default())
            .context("server login finish")?;

        let session_key = server_finish.session_key.as_slice().to_vec();
        info!("client authenticated via OPAQUE");
        Ok(session_key)
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub async fn client_register(
    password: &[u8],
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
) -> Result<()> {
    let mut rng = OsRng;

    let client_start = ClientRegistration::<MewCipherSuite>::start(&mut rng, password)
        .context("client registration start")?;
    let req_bytes: opaque_ke::generic_array::GenericArray<_, _> = client_start.message.serialize();
    send_frame(writer, &req_bytes).await?;

    let resp_bytes = recv_frame(reader).await?;
    let resp = RegistrationResponse::deserialize(&resp_bytes)
        .context("deserialize registration response")?;

    let client_finish = client_start
        .state
        .finish(
            &mut rng,
            password,
            resp,
            ClientRegistrationFinishParameters::default(),
        )
        .context("client registration finish")?;
    let upload_bytes: opaque_ke::generic_array::GenericArray<_, _> =
        client_finish.message.serialize();
    send_frame(writer, &upload_bytes).await?;

    info!("client registered with server");
    Ok(())
}

pub async fn client_login(
    password: &[u8],
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
) -> Result<Vec<u8>> {
    let mut rng = OsRng;

    let client_start =
        ClientLogin::<MewCipherSuite>::start(&mut rng, password).context("client login start")?;
    let req_bytes: opaque_ke::generic_array::GenericArray<_, _> = client_start.message.serialize();
    send_frame(writer, &req_bytes).await?;

    let resp_bytes = recv_frame(reader).await?;
    if resp_bytes.is_empty() {
        anyhow::bail!("server rejected login (no matching registration)");
    }
    let resp =
        CredentialResponse::deserialize(&resp_bytes).context("deserialize credential response")?;

    let client_finish = client_start
        .state
        .finish(
            &mut rng,
            password,
            resp,
            ClientLoginFinishParameters::default(),
        )
        .context("client login finish")?;
    let fin_bytes: opaque_ke::generic_array::GenericArray<_, _> = client_finish.message.serialize();
    send_frame(writer, &fin_bytes).await?;

    let session_key = client_finish.session_key.as_slice().to_vec();
    info!("client authenticated, session key established");
    Ok(session_key)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{BufReader, BufWriter};

    #[tokio::test]
    async fn test_opaque_register_and_login() {
        let (cr, sw) = tokio::io::duplex(4096);
        let (sr, cw) = tokio::io::duplex(4096);

        let password = b"123-456";

        let server_handle = tokio::spawn(async move {
            let mut server = PairingServer::new();
            let mut reader = BufReader::new(sr);
            let mut writer = BufWriter::new(sw);

            server.register(&mut reader, &mut writer).await.unwrap();
            let session_key = server.login(&mut reader, &mut writer).await.unwrap();
            session_key
        });

        let client_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(cr);
            let mut writer = BufWriter::new(cw);

            client_register(password, &mut reader, &mut writer)
                .await
                .unwrap();
            let session_key = client_login(password, &mut reader, &mut writer)
                .await
                .unwrap();
            session_key
        });

        let server_key = server_handle.await.unwrap();
        let client_key = client_handle.await.unwrap();
        assert_eq!(server_key, client_key, "session keys must match");
        assert!(!server_key.is_empty(), "session key must not be empty");
    }

    #[tokio::test]
    async fn test_opaque_login_wrong_password_fails() {
        let (cr, sw) = tokio::io::duplex(4096);
        let (sr, cw) = tokio::io::duplex(4096);

        let register_password = b"123-456";
        let login_password = b"999-999";

        let server_handle = tokio::spawn(async move {
            let mut server = PairingServer::new();
            let mut reader = BufReader::new(sr);
            let mut writer = BufWriter::new(sw);

            server.register(&mut reader, &mut writer).await.unwrap();
            let result = server.login(&mut reader, &mut writer).await;
            assert!(result.is_err());
        });

        let client_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(cr);
            let mut writer = BufWriter::new(cw);

            client_register(register_password, &mut reader, &mut writer)
                .await
                .unwrap();
            let result = client_login(login_password, &mut reader, &mut writer).await;
            assert!(result.is_err());
        });

        let _ = tokio::join!(server_handle, client_handle);
    }
}
