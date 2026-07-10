//! In-tree OpenAI/Codex OAuth: PKCE browser flow + headless device flow.
//!
//! Replaces the external `openai-auth` crate so we own the entire auth path.
//! Two motivations:
//! - `openai-auth` opens URLs via the `webbrowser` crate, which honors
//!   `$BROWSER` first; if that points at an editor, login opens an editor.
//!   We open via the platform launcher (`open`/`xdg-open`/`start`) and never
//!   consult `$BROWSER`.
//! - A device-code flow lets users authenticate on headless machines or from
//!   a phone while the CLI runs on a remote box.
//!
//! Protocol references (grounded in vendored `openai-auth-1.0.0` and the
//! opencode `codex.ts` source):
//! - Browser PKCE: authorize at `{issuer}/oauth/authorize` (S256 challenge),
//!   exchange at `{issuer}/oauth/token` (form-encoded).
//! - Device flow: POST `{issuer}/api/accounts/deviceauth/usercode` for a code,
//!   poll `{issuer}/api/accounts/deviceauth/token` (403/404 = pending), then
//!   exchange the returned code+verifier at the normal token endpoint with
//!   `redirect_uri = {issuer}/deviceauth/callback`.

use anyhow::{Context, Result};
use base64::Engine;
use sha2::{Digest, Sha256};

const PROD_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const PROD_ISSUER: &str = "https://auth.openai.com";
/// Registered redirect URI for the browser flow. Byte-exact: both the
/// authorize URL and the token-exchange form must send this exact string.
pub(crate) const PROD_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const POLL_MARGIN_MS: u64 = 3000;

pub(crate) struct OAuthConfig {
    pub(crate) client_id: String,
    /// Issuer base URL; all endpoints are derived from it. Tests override
    /// this with a mock server URI.
    pub(crate) issuer: String,
    /// Extra milliseconds added to the device-poll sleep interval to avoid
    /// hammering the server. 3000 in production, tiny in tests.
    pub(crate) poll_margin_ms: u64,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            client_id: PROD_CLIENT_ID.to_string(),
            issuer: PROD_ISSUER.to_string(),
            poll_margin_ms: POLL_MARGIN_MS,
        }
    }
}

impl OAuthConfig {
    fn base(&self) -> &str {
        self.issuer.trim_end_matches('/')
    }
    pub(crate) fn token_url(&self) -> String {
        format!("{}/oauth/token", self.base())
    }
    pub(crate) fn device_usercode_url(&self) -> String {
        format!("{}/api/accounts/deviceauth/usercode", self.base())
    }
    pub(crate) fn device_token_url(&self) -> String {
        format!("{}/api/accounts/deviceauth/token", self.base())
    }
    pub(crate) fn device_page_url(&self) -> String {
        format!("{}/codex/device", self.base())
    }
    pub(crate) fn device_redirect_uri(&self) -> String {
        format!("{}/deviceauth/callback", self.base())
    }
}

// ---------------------------------------------------------------------------
// PKCE + state
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct PkcePair {
    pub(crate) verifier: String,
    pub(crate) challenge: String,
}

/// Generate a PKCE pair: 32 random bytes → b64url-no-pad verifier (43 chars),
/// challenge = b64url-no-pad(sha256(verifier)).
pub(crate) fn generate_pkce() -> PkcePair {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let challenge = pkce_challenge(&verifier);
    PkcePair {
        verifier,
        challenge,
    }
}

/// Compute the S256 challenge for a verifier. Split out so the RFC 7636
/// test vector can be checked independently of generation.
pub(crate) fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Random 32-byte state token for CSRF protection.
pub(crate) fn generate_state() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn build_authorize_url(
    cfg: &OAuthConfig,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Result<String> {
    let url = reqwest::Url::parse_with_params(
        &format!("{}/oauth/authorize", cfg.base()),
        &[
            ("response_type", "code"),
            ("client_id", cfg.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", "openid profile email offline_access"),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "codex_cli_rs"),
        ],
    )?;
    Ok(url.to_string())
}

// ---------------------------------------------------------------------------
// JWT account-id extraction (no signature check)
// ---------------------------------------------------------------------------

/// Extract the `chatgpt_account_id` from an access_token JWT's
/// `https://api.openai.com/auth` claim. The signature is not verified — we
/// trust tokens obtained from our own OAuth exchange.
pub(crate) fn extract_account_id(access_token: &str) -> Result<String> {
    let mut parts = access_token.split('.');
    let _header = parts.next();
    let payload = parts
        .next()
        .context("malformed JWT: missing payload segment")?;
    // JWT payloads are base64url without padding; strip any stray '=' so the
    // no-pad engine accepts them.
    let stripped = payload.trim_end_matches('=');
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(stripped)
        .context("malformed JWT: payload is not valid base64url")?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).context("malformed JWT: payload is not valid JSON")?;
    let claim = value
        .get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .context("JWT missing claim https://api.openai.com/auth.chatgpt_account_id")?;
    Ok(claim.to_string())
}

// ---------------------------------------------------------------------------
// Token exchange + refresh
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct OAuthTokens {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_at: u64,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl TokenResponse {
    /// Convert the wire response into our token set. `fallback_refresh` is
    /// reused when the response omits `refresh_token` (refresh grants often
    /// do).
    fn into_tokens(self, fallback_refresh: Option<&str>) -> OAuthTokens {
        let refresh_token = self
            .refresh_token
            .or_else(|| fallback_refresh.map(str::to_string))
            .unwrap_or_default();
        let expires_at = now_secs() + self.expires_in.unwrap_or(3600);
        OAuthTokens {
            access_token: self.access_token,
            refresh_token,
            expires_at,
        }
    }
}

pub(crate) async fn exchange_code(
    cfg: &OAuthConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokens> {
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", cfg.client_id.as_str()),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
    ];
    let resp = client
        .post(cfg.token_url())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("token exchange failed (HTTP {status}): {body}");
    }
    let tr: TokenResponse = resp.json().await.context("parsing token response")?;
    Ok(tr.into_tokens(None))
}

pub(crate) async fn refresh_tokens(cfg: &OAuthConfig, refresh_token: &str) -> Result<OAuthTokens> {
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", cfg.client_id.as_str()),
    ];
    let resp = client
        .post(cfg.token_url())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("token refresh failed (HTTP {status}): {body}");
    }
    let tr: TokenResponse = resp.json().await.context("parsing token response")?;
    Ok(tr.into_tokens(Some(refresh_token)))
}

// ---------------------------------------------------------------------------
// Browser flow: local callback server + platform launcher
// ---------------------------------------------------------------------------

pub(crate) struct CallbackServer {
    listener: tokio::net::TcpListener,
}

impl CallbackServer {
    /// Bind a one-shot callback server on 127.0.0.1. Bind before opening the
    /// browser so an in-use port fails fast rather than mid-flow.
    pub(crate) async fn bind(port: u16) -> Result<Self> {
        let addr = format!("127.0.0.1:{port}");
        let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
            let hint = if port == 1455 {
                " — port 1455 is in use; try `mew auth login --headless`"
            } else {
                ""
            };
            anyhow::anyhow!("failed to bind callback server on {addr}: {e}{hint}")
        })?;
        Ok(Self { listener })
    }

    /// The bound port — only meaningful for port-0 tests that need to
    /// discover the ephemeral port the OS assigned.
    #[cfg(test)]
    pub(crate) fn port(&self) -> u16 {
        self.listener.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Accept connections until a callback resolves the code (or an error).
    /// Non-callback paths (favicon, etc.) get a 404 and the loop continues.
    /// The 120s timeout is applied by the caller.
    pub(crate) async fn wait_for_code(self, expected_state: &str) -> Result<String> {
        let Self { listener } = self;
        let expected = expected_state.to_string();
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("callback server accept error: {e}");
                    continue;
                }
            };
            // Read the first chunk; the request line lives at the top. Cap at
            // 8 KiB so a pathological client can't grow memory unbounded.
            let mut buf = [0u8; 8192];
            let n = match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("callback server read error: {e}");
                    continue;
                }
            };
            let request_data = std::str::from_utf8(&buf[..n]).unwrap_or("");
            let request_line = request_data.lines().next().unwrap_or("").trim_end();
            let result = handle_callback_line(request_line, &expected);
            write_response(&mut stream, &result).await;
            match result.terminal {
                Some(Ok(code)) => return Ok(code),
                Some(Err(msg)) => return Err(anyhow::anyhow!(msg)),
                None => continue,
            }
        }
    }
}

struct CallbackResult {
    status: u16,
    reason: &'static str,
    body: String,
    /// `None` = keep listening (e.g. favicon 404). `Some(Ok)` → return code.
    /// `Some(Err)` → terminate with an error.
    terminal: Option<std::result::Result<String, String>>,
}

fn not_found() -> CallbackResult {
    CallbackResult {
        status: 404,
        reason: "Not Found",
        body: "Not Found".to_string(),
        terminal: None,
    }
}

fn handle_callback_line(request_line: &str, expected_state: &str) -> CallbackResult {
    let mut parts = request_line.split_whitespace();
    let _method = parts.next();
    let target = parts.next().unwrap_or("/");
    let url = match reqwest::Url::parse(&format!("http://localhost{target}")) {
        Ok(u) => u,
        Err(_) => return not_found(),
    };

    if url.path() != "/auth/callback" {
        return not_found();
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            "error_description" => error_description = Some(v.into_owned()),
            _ => {}
        }
    }

    if let Some(err) = error {
        let msg = match error_description {
            Some(d) if !d.is_empty() => format!("OAuth error: {err} — {d}"),
            _ => format!("OAuth error: {err}"),
        };
        return CallbackResult {
            status: 200,
            reason: "OK",
            body: error_page(&msg),
            terminal: Some(Err(msg)),
        };
    }

    let received_state = state.as_deref().unwrap_or("");
    if received_state != expected_state {
        let msg = "state mismatch — possible CSRF attack".to_string();
        return CallbackResult {
            status: 400,
            reason: "Bad Request",
            body: error_page(&msg),
            terminal: Some(Err(msg)),
        };
    }

    match code {
        Some(c) if !c.is_empty() => CallbackResult {
            status: 200,
            reason: "OK",
            body: success_page(),
            terminal: Some(Ok(c)),
        },
        _ => {
            let msg = "missing authorization code".to_string();
            CallbackResult {
                status: 400,
                reason: "Bad Request",
                body: error_page(&msg),
                terminal: Some(Err(msg)),
            }
        }
    }
}

async fn write_response(stream: &mut tokio::net::TcpStream, result: &CallbackResult) {
    use tokio::io::AsyncWriteExt;
    let body = result.body.as_bytes();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        result.status,
        result.reason,
        body.len()
    );
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    let _ = stream.write_all(&out).await;
    let _ = stream.flush().await;
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn error_page(msg: &str) -> String {
    format!(
        "<html><body><h1>Authorization failed</h1><p>{}</p>\
         <p>You can close this window.</p></body></html>",
        html_escape(msg)
    )
}

fn success_page() -> String {
    "<html><body><h1>Authorization successful</h1>\
     <p>You can close this window and return to the terminal.</p></body></html>"
        .to_string()
}

/// Open a URL in the user's default browser via the platform launcher.
///
/// NEVER consults `$BROWSER` — that was the root cause of the editor-opening
/// bug in `openai-auth`/`webbrowser`. Failure to launch is non-fatal; the
/// caller already printed the URL for manual copy.
pub(crate) fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).status()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).status()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Headless flow: device code
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct DeviceCode {
    pub(crate) device_auth_id: String,
    pub(crate) user_code: String,
    /// Poll interval in seconds (clamped to >= 1).
    pub(crate) interval: u64,
}

#[derive(serde::Deserialize)]
struct DeviceCodeResponse {
    device_auth_id: String,
    user_code: String,
    /// The API returns this as a string (opencode `parseInt`s it); accept
    /// either a number or a string to be robust.
    #[serde(default)]
    interval: serde_json::Value,
}

pub(crate) async fn request_device_code(cfg: &OAuthConfig) -> Result<DeviceCode> {
    let client = reqwest::Client::new();
    let resp = client
        .post(cfg.device_usercode_url())
        .header("Content-Type", "application/json")
        .header("User-Agent", user_agent())
        .json(&serde_json::json!({ "client_id": cfg.client_id }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("device code request failed (HTTP {status}): {body}");
    }
    let parsed: DeviceCodeResponse = resp.json().await.context("parsing device code response")?;
    let interval = match parsed.interval {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(5),
        serde_json::Value::String(s) => s.parse().unwrap_or(5),
        _ => 5,
    };
    Ok(DeviceCode {
        device_auth_id: parsed.device_auth_id,
        user_code: parsed.user_code,
        interval: interval.max(1),
    })
}

#[derive(serde::Deserialize)]
struct DeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

/// Poll the device-token endpoint until the user completes authorization.
/// Returns `(authorization_code, code_verifier)` — the server holds the PKCE
/// verifier for the device flow, so the caller exchanges with that, not its
/// own. 403/404 means pending (sleep and retry); any other non-2xx fails.
pub(crate) async fn poll_device_token(
    cfg: &OAuthConfig,
    dc: &DeviceCode,
) -> Result<(String, String)> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "device_auth_id": dc.device_auth_id,
        "user_code": dc.user_code,
    });
    loop {
        let resp = client
            .post(cfg.device_token_url())
            .header("Content-Type", "application/json")
            .header("User-Agent", user_agent())
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            let parsed: DeviceTokenResponse =
                resp.json().await.context("parsing device token response")?;
            return Ok((parsed.authorization_code, parsed.code_verifier));
        }
        let code = status.as_u16();
        if code == 403 || code == 404 {
            tokio::time::sleep(std::time::Duration::from_millis(
                dc.interval * 1000 + cfg.poll_margin_ms,
            ))
            .await;
            continue;
        }
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("device token poll failed (HTTP {code}): {text}");
    }
}

fn user_agent() -> String {
    format!("mew/{}", env!("CARGO_PKG_VERSION"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_cfg(issuer: &str) -> OAuthConfig {
        OAuthConfig {
            client_id: PROD_CLIENT_ID.to_string(),
            issuer: issuer.to_string(),
            poll_margin_ms: 10,
        }
    }

    fn fake_jwt(payload_json: &str) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        format!("{header}.{payload}.sig")
    }

    // --- PKCE / state / authorize URL ---

    #[test]
    fn pkce_challenge_matches_rfc_7636_test_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = pkce_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_pkce_is_well_formed_and_unique() {
        let a = generate_pkce();
        let b = generate_pkce();
        assert_eq!(
            a.verifier.len(),
            43,
            "verifier must be 43 chars (32 bytes b64url)"
        );
        assert!(!a.verifier.contains('='), "verifier must be unpadded");
        assert_eq!(
            pkce_challenge(&a.verifier),
            a.challenge,
            "challenge must match verifier"
        );
        assert_ne!(a.verifier, b.verifier, "two generations must differ");
    }

    #[test]
    fn generated_state_is_unique() {
        let a = generate_state();
        let b = generate_state();
        assert!(!a.is_empty());
        assert_ne!(a, b);
    }

    #[test]
    fn build_authorize_url_contains_all_params() {
        let cfg = OAuthConfig::default();
        let url = build_authorize_url(&cfg, PROD_REDIRECT_URI, "thechallenge", "thestate").unwrap();
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        // redirect_uri must be URL-encoded in the query string.
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        assert!(url.contains("scope=openid+profile+email+offline_access"));
        assert!(url.contains("code_challenge=thechallenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=thestate"));
        assert!(url.contains("id_token_add_organizations=true"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("originator=codex_cli_rs"));
    }

    // --- JWT extraction ---

    #[test]
    fn extract_account_id_from_fake_jwt() {
        let payload = serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_42" }
        })
        .to_string();
        let token = fake_jwt(&payload);
        assert_eq!(extract_account_id(&token).unwrap(), "acct_42");
    }

    #[test]
    fn extract_account_id_missing_claim_returns_err() {
        let payload = serde_json::json!({"sub": "x"}).to_string();
        let token = fake_jwt(&payload);
        let err = extract_account_id(&token).unwrap_err();
        assert!(err.to_string().contains("chatgpt_account_id"), "{}", err);
    }

    #[test]
    fn extract_account_id_malformed_jwt_returns_err() {
        assert!(extract_account_id("").is_err());
        assert!(extract_account_id("onlyone").is_err());
        assert!(extract_account_id("not.a.jwt").is_err());
    }

    // --- token exchange / refresh (wiremock) ---

    #[tokio::test]
    async fn exchange_code_sends_form_fields_and_parses_tokens() {
        let server = MockServer::start().await;
        let cfg = test_cfg(&server.uri());

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains(
                "client_id=app_EMoamEEZ73f0CkXaXp7hrann",
            ))
            .and(body_string_contains("code=mycode"))
            .and(body_string_contains("code_verifier=myverifier"))
            .and(body_string_contains("redirect_uri="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at",
                "refresh_token": "rt",
                "expires_in": 7200,
            })))
            .mount(&server)
            .await;

        let tokens = exchange_code(
            &cfg,
            "mycode",
            "myverifier",
            "http://localhost:1455/auth/callback",
        )
        .await
        .unwrap();
        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.refresh_token, "rt");
        let now = now_secs();
        assert!(
            tokens.expires_at >= now + 7190 && tokens.expires_at <= now + 7210,
            "expires_at should be ~now+7200, got {} (now {})",
            tokens.expires_at,
            now
        );
    }

    #[tokio::test]
    async fn exchange_code_defaults_expires_in_to_3600() {
        let server = MockServer::start().await;
        let cfg = test_cfg(&server.uri());

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"access_token": "at", "refresh_token": "rt"}),
                ),
            )
            .mount(&server)
            .await;

        let tokens = exchange_code(&cfg, "c", "v", "http://x/").await.unwrap();
        let now = now_secs();
        assert!(
            tokens.expires_at >= now + 3590 && tokens.expires_at <= now + 3610,
            "expires_at should default to ~now+3600"
        );
    }

    #[tokio::test]
    async fn refresh_sends_refresh_grant_and_reuses_omitted_refresh_token() {
        let server = MockServer::start().await;
        let cfg = test_cfg(&server.uri());

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=oldrt"))
            .and(body_string_contains(
                "client_id=app_EMoamEEZ73f0CkXaXp7hrann",
            ))
            // Response omits refresh_token → caller must reuse the old one.
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"access_token": "newat"})),
            )
            .mount(&server)
            .await;

        let tokens = refresh_tokens(&cfg, "oldrt").await.unwrap();
        assert_eq!(tokens.access_token, "newat");
        assert_eq!(
            tokens.refresh_token, "oldrt",
            "must reuse old refresh_token when omitted"
        );
    }

    #[tokio::test]
    async fn exchange_code_surfaces_non_2xx_status_and_body() {
        let server = MockServer::start().await;
        let cfg = test_cfg(&server.uri());

        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(400).set_body_string("invalid_grant: bad code"))
            .mount(&server)
            .await;

        let err = exchange_code(&cfg, "bad", "v", "http://x/")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("400"), "msg: {msg}");
        assert!(msg.contains("invalid_grant"), "msg: {msg}");
    }

    // --- device flow (wiremock) ---

    #[tokio::test]
    async fn request_device_code_sends_json_client_id_and_headers() {
        let server = MockServer::start().await;
        let cfg = test_cfg(&server.uri());

        Mock::given(method("POST"))
            .and(path("/api/accounts/deviceauth/usercode"))
            .and(header("content-type", "application/json"))
            .and(body_partial_json(serde_json::json!({
                "client_id": "app_EMoamEEZ73f0CkXaXp7hrann"
            })))
            // The API returns interval as a string (opencode parseInts it).
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_auth_id": "daid",
                "user_code": "ABC-123",
                "interval": "1"
            })))
            .mount(&server)
            .await;

        let dc = request_device_code(&cfg).await.unwrap();
        assert_eq!(dc.device_auth_id, "daid");
        assert_eq!(dc.user_code, "ABC-123");
        assert_eq!(dc.interval, 1, "string interval must be parsed");
    }

    #[tokio::test]
    async fn request_device_code_accepts_numeric_interval() {
        let server = MockServer::start().await;
        let cfg = test_cfg(&server.uri());

        Mock::given(method("POST"))
            .and(path("/api/accounts/deviceauth/usercode"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_auth_id": "d",
                "user_code": "U",
                "interval": 2
            })))
            .mount(&server)
            .await;

        let dc = request_device_code(&cfg).await.unwrap();
        assert_eq!(dc.interval, 2);
    }

    #[tokio::test]
    async fn poll_device_token_retries_on_403_then_returns_codes() {
        let server = MockServer::start().await;
        let cfg = test_cfg(&server.uri());

        // Pending responses (403) up to twice, then success.
        Mock::given(method("POST"))
            .and(path("/api/accounts/deviceauth/token"))
            .respond_with(ResponseTemplate::new(403))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/accounts/deviceauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "authorization_code": "theauthcode",
                "code_verifier": "serververifier"
            })))
            .mount(&server)
            .await;

        let dc = DeviceCode {
            device_auth_id: "daid".into(),
            user_code: "UC".into(),
            interval: 1,
        };
        let (code, verifier) = poll_device_token(&cfg, &dc).await.unwrap();
        assert_eq!(code, "theauthcode");
        assert_eq!(verifier, "serververifier");
    }

    #[tokio::test]
    async fn poll_device_token_500_returns_err() {
        let server = MockServer::start().await;
        let cfg = test_cfg(&server.uri());

        Mock::given(method("POST"))
            .and(path("/api/accounts/deviceauth/token"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let dc = DeviceCode {
            device_auth_id: "d".into(),
            user_code: "u".into(),
            interval: 1,
        };
        let err = poll_device_token(&cfg, &dc).await.unwrap_err();
        assert!(err.to_string().contains("500"), "msg: {}", err);
    }

    // --- callback server (real localhost, port 0) ---

    #[tokio::test]
    async fn callback_happy_path_returns_code() {
        let server = CallbackServer::bind(0).await.unwrap();
        let port = server.port();
        let task = tokio::spawn(async move { server.wait_for_code("st").await });

        let resp = reqwest::get(format!(
            "http://127.0.0.1:{port}/auth/callback?code=abc&state=st"
        ))
        .await
        .unwrap();
        assert!(resp.status().is_success());

        let code = task.await.unwrap().unwrap();
        assert_eq!(code, "abc");
    }

    #[tokio::test]
    async fn callback_state_mismatch_returns_400_and_err() {
        let server = CallbackServer::bind(0).await.unwrap();
        let port = server.port();
        let task = tokio::spawn(async move { server.wait_for_code("expected").await });

        let resp = reqwest::get(format!(
            "http://127.0.0.1:{port}/auth/callback?code=abc&state=wrong"
        ))
        .await
        .unwrap();
        assert_eq!(resp.status().as_u16(), 400);

        let err = task.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("state mismatch"), "msg: {}", err);
    }

    #[tokio::test]
    async fn callback_error_param_returns_err_with_decoded_description() {
        let server = CallbackServer::bind(0).await.unwrap();
        let port = server.port();
        let task = tokio::spawn(async move { server.wait_for_code("st").await });

        // error_description is percent-encoded; the server must decode it.
        let resp = reqwest::get(format!(
            "http://127.0.0.1:{port}/auth/callback?error=access_denied&error_description=user%20declined%20it"
        ))
        .await
        .unwrap();
        assert!(resp.status().is_success(), "error page returns 200");

        let err = task.await.unwrap().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("access_denied"), "msg: {msg}");
        assert!(
            msg.contains("user declined it"),
            "must contain decoded description: {msg}"
        );
    }

    #[tokio::test]
    async fn callback_favicon_returns_404_and_keeps_waiting() {
        let server = CallbackServer::bind(0).await.unwrap();
        let port = server.port();
        let task = tokio::spawn(async move { server.wait_for_code("st").await });

        let fav = reqwest::get(format!("http://127.0.0.1:{port}/favicon.ico"))
            .await
            .unwrap();
        assert_eq!(fav.status().as_u16(), 404);

        // Server must still be waiting — send a real callback.
        let resp = reqwest::get(format!(
            "http://127.0.0.1:{port}/auth/callback?code=ok&state=st"
        ))
        .await
        .unwrap();
        assert!(resp.status().is_success());

        let code = task.await.unwrap().unwrap();
        assert_eq!(code, "ok");
    }

    #[tokio::test]
    async fn callback_no_request_times_out() {
        let server = CallbackServer::bind(0).await.unwrap();
        let res = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            server.wait_for_code("st"),
        )
        .await;
        assert!(
            res.is_err(),
            "outer timeout must fire when no request arrives"
        );
    }
}
