# own the openai/codex oauth: replace `openai-auth` with in-tree PKCE + device flow

## Context

`mew auth login` (ChatGPT/Codex OAuth) opens "some weird editor" instead of a browser.
Root cause: the external `openai-auth` crate (v1.0.0) opens URLs via the `webbrowser`
crate, which honors `$BROWSER` before anything else — if that's set to an editor, you
get an editor. We also don't want a third-party crate owning our auth path. All usage
is confined to `crates/mew-provider-responses/src/oauth.rs` (4 call sites), so we
replace the library with ~350 lines we own, and add a headless device-code flow
(user decision: browser + headless).

The generic engine (`crates/mew-provider/src/auth.rs`: `OAuthProvider` trait, token
storage at `~/.config/mew/auth/openai-responses.json`, `refresh_if_needed`) stays
unchanged except for one trait-signature addition (headless flag).

## Protocol facts (grounded, not from memory)

From vendored `openai-auth-1.0.0` source (`~/.cargo/registry/src/.../openai-auth-1.0.0/`):
- client_id `app_EMoamEEZ73f0CkXaXp7hrann`, issuer `https://auth.openai.com`
- authorize: `{issuer}/oauth/authorize` with `response_type=code`, `client_id`,
  `redirect_uri=http://localhost:1455/auth/callback`, `scope=openid profile email
  offline_access`, `code_challenge` (S256), `code_challenge_method=S256`, `state`,
  `id_token_add_organizations=true`, `codex_cli_simplified_flow=true`, `originator=codex_cli_rs`
- token: POST `{issuer}/oauth/token` form-encoded. exchange: `grant_type=authorization_code,
  client_id, code, code_verifier, redirect_uri`; refresh: `grant_type=refresh_token,
  refresh_token, client_id`. response `{access_token, id_token?, refresh_token?, expires_in?}`
  (default 3600)
- PKCE: 32 random bytes → b64url-no-pad verifier (43 chars); challenge = b64url-no-pad(sha256(verifier))
- account id: decode access_token JWT payload (no signature check), claim
  `https://api.openai.com/auth` → `chatgpt_account_id`. Sent as `chatgpt-account-id` header.

From opencode source (fetched from github, packages/opencode/src/plugin/openai/codex.ts):
- device start: POST `{issuer}/api/accounts/deviceauth/usercode`, JSON `{client_id}`,
  `Content-Type: application/json` + `User-Agent` → `{device_auth_id, user_code, interval}`
- user visits `{issuer}/codex/device` and enters `user_code`
- poll: POST `{issuer}/api/accounts/deviceauth/token`, JSON `{device_auth_id, user_code}`.
  403/404 = pending → sleep `interval + 3s` margin, retry. 200 → `{authorization_code,
  code_verifier}` (server-held PKCE). any other status → fail.
- then normal exchange at `{issuer}/oauth/token` with that code + code_verifier and
  `redirect_uri = {issuer}/deviceauth/callback`

## Files

- **new** `crates/mew-provider-responses/src/openai_oauth.rs` — all protocol machinery,
  `pub(crate)`, tests at bottom (crate convention: large single file + `#[cfg(test)]`)
- `crates/mew-provider-responses/src/oauth.rs` — rewire `OAuthProvider` impl, minimal diff
- `crates/mew-provider-responses/src/lib.rs` — add `mod openai_oauth;`
- `crates/mew-provider/src/auth.rs` — `login()` gains a headless flag (see below)
- `crates/mew/src/cli.rs` + `crates/mew/src/commands/auth.rs` — `--headless` flag on `auth login`
- `crates/mew-provider-responses/Cargo.toml` + workspace `Cargo.toml` — dep swap

## Internal API (`openai_oauth.rs`)

```rust
pub(crate) struct OAuthConfig {         // Default = production values
    client_id: String,
    issuer: String,                     // endpoints derived; tests override with mock uri
    poll_margin_ms: u64,                // 3000 in prod, ~10 in tests
}
pub(crate) struct PkcePair { verifier: String, challenge: String }
pub(crate) fn generate_pkce() -> PkcePair;              // rand OsRng + sha2 + base64 URL_SAFE_NO_PAD
pub(crate) fn pkce_challenge(verifier: &str) -> String; // split out for RFC 7636 test vector
pub(crate) fn generate_state() -> String;
pub(crate) fn build_authorize_url(cfg, redirect_uri, challenge, state) -> String; // reqwest::Url::parse_with_params
pub(crate) struct OAuthTokens { access_token, refresh_token, expires_at: u64 }
pub(crate) async fn exchange_code(cfg, code, verifier, redirect_uri) -> anyhow::Result<OAuthTokens>;
pub(crate) async fn refresh_tokens(cfg, refresh_token) -> anyhow::Result<OAuthTokens>;
    // if response omits refresh_token, reuse the one passed in
pub(crate) fn extract_account_id(access_token: &str) -> anyhow::Result<String>;
    // manual: split('.').nth(1), strip '=' padding, b64url decode, serde_json — no jsonwebtoken

// browser flow
pub(crate) struct CallbackServer;       // tokio TcpListener on 127.0.0.1
impl CallbackServer {
    async fn bind(port: u16) -> anyhow::Result<Self>;  // bind BEFORE opening browser; AddrInUse → "port 1455 in use" msg
    fn port(&self) -> u16;                              // for port-0 tests
    async fn wait_for_code(self, expected_state: &str) -> anyhow::Result<String>;
}
pub(crate) fn open_browser(url: &str) -> std::io::Result<()>;
    // macOS `open`, linux `xdg-open`, windows `cmd /C start "" <url>`; NEVER $BROWSER; non-fatal

// headless flow
pub(crate) struct DeviceCode { device_auth_id, user_code, interval: u64 }
pub(crate) async fn request_device_code(cfg) -> anyhow::Result<DeviceCode>;
pub(crate) async fn poll_device_token(cfg, &DeviceCode) -> anyhow::Result<(String, String)>;
    // (authorization_code, code_verifier); 403/404 → sleep(interval + margin) and retry; other → bail
```

Callback server details: one-shot accept loop, parse request line only (≤8 KiB),
`/auth/callback` with `code`+matching `state` → 200 success HTML, return code;
`error` param → error page + Err with `error`/`error_description` (needs a ~15-line
`percent_decode`); state mismatch → 400 + Err; other paths (favicon) → 404, keep looping.
The 120s timeout stays at the call site in `oauth.rs`.

## Rewiring

`oauth.rs::login(headless)`:
- browser path: `bind(1455)` → pkce/state → `build_authorize_url` → existing eprintlns
  (always print URL) → `let _ = open_browser(url)` → `timeout(120s, wait_for_code)` →
  `exchange_code` → `extract_account_id` → `OAuthSession` (same shape as today)
- headless path: `request_device_code` → eprintln "visit {issuer}/codex/device and enter
  code: {user_code}" → `timeout(900s, poll_device_token)` → `exchange_code` with
  `redirect_uri = {issuer}/deviceauth/callback` and the server-supplied verifier →
  same tail. `User-Agent: mew/<CARGO_PKG_VERSION>` on device endpoints.
- `extra_headers()` / `refresh()`: swap `openai_auth::` calls for `crate::openai_oauth::`,
  drop the pointless client construction.

Trait change (`mew-provider/src/auth.rs`): `async fn login(&self, headless: bool)`;
`pub async fn login(provider, headless)` threads it through. Single impl + single caller,
so this is a two-line ripple.

CLI: `AuthCommands::Login { provider, #[arg(long)] headless: bool }` in
`crates/mew/src/cli.rs:301`; pass through in `commands/auth.rs::auth_cmd`. If `bind(1455)`
fails in browser mode, the error hints at `--headless`.

## Cargo changes

- workspace `Cargo.toml`: remove `openai-auth = "1"` (line 149); add `base64 = "0.22"`,
  `sha2 = "0.10"` to workspace deps (both already in Cargo.lock transitively)
- `mew-provider-responses/Cargo.toml`: remove `openai-auth`; add `base64`, `sha2`,
  `rand` (workspace). reqwest/tokio/serde/anyhow already present; wiremock in dev-deps.
- lockfile after build drops `openai-auth`, `webbrowser`, `tiny_http`, `jsonwebtoken`,
  `querystring`. Verify with `grep openai-auth Cargo.lock` → empty.

## Test plan (TDD — each group written red first)

Unit (in `openai_oauth.rs`):
1. RFC 7636 vector: verifier `dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` →
   challenge `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM`
2. generated verifier is 43 chars, challenge consistent, two calls differ; state likewise
3. authorize URL contains every param incl. the three codex-specific ones + encoded redirect_uri
4. `extract_account_id`: fake unsigned JWT with the claim → id; malformed token and
   missing claim → Err
5. `percent_decode`

Wiremock (`#[tokio::test]`):
6. exchange sends exact form fields (grant_type, client_id, code_verifier, redirect_uri);
   `expires_at` ≈ now+expires_in; default 3600 when omitted
7. refresh sends refresh grant; reuses old refresh_token when response omits it
8. non-2xx token response surfaces status+body in Err
9. device: usercode request sends JSON client_id + headers; poll returns 403, 403, then 200
   (wiremock `.up_to_n_times(2)`) → codes returned; poll 500 → Err; tiny `poll_margin_ms`

Callback server (real localhost, port 0):
10. happy path (code+state → 200 page, task yields code); state mismatch → 400 + Err;
    `error=access_denied&error_description=...` → Err containing decoded description;
    `/favicon.ico` → 404 and server keeps waiting; no request + outer timeout → Elapsed

`oauth.rs`: keep existing 3 tests; add `extra_headers` extracts account id from fake JWT /
empty on bad token.

## Sequencing

1. stub `openai_oauth.rs` + unit tests (red) → implement pure fns (green)
2. wiremock token tests (red) → `exchange_code`/`refresh_tokens`
3. callback-server tests (red) → `CallbackServer`; then `open_browser` (cfg-gated, compile-only)
4. device-flow tests (red) → `request_device_code`/`poll_device_token`
5. rewire `oauth.rs` + trait/CLI headless flag + their tests
6. Cargo.toml swaps, rebuild lock, `grep openai-auth Cargo.lock` empty
7. `just ci` (fmt, clippy -D warnings, arch-check, tests)
8. CURRENT.md dated section

## Verification

- unit + wiremock + localhost-server tests above (no live-API tests possible for oauth)
- e2e smoke on this mac: `MEW_CONFIG_DIR`-isolated? no — real run: `cargo run -p mew -- auth
  login` with `$BROWSER=vim` exported to prove the bug is gone (browser opens via `open`,
  vim never appears), complete login, check `~/.config/mew/auth/openai-responses.json`
  perms 0600, then `mew auth status` and one chat turn against gpt-5-codex
- `mew auth login --headless`: verify code + URL print, complete on phone, tokens land

## Watch-outs

- clippy `-D warnings`: no unwrap outside tests; keep `use std::process::Command` inside
  cfg blocks so non-target builds don't warn
- prod redirect_uri must be byte-exact `http://localhost:1455/auth/callback` in both
  authorize URL and exchange form (registered value); only tests deviate
- base64 padding tolerance when decoding JWT payloads (strip trailing `=`)
- future work noted in module doc: none — both flows now in scope
