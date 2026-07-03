# mew accounts, roaming & one-click environments — plan

Rougher-cut plan (outline + key decisions, less line-level than the jj/misc
specs) for optional centralized accounts: multi-device access, secret sync,
and one-click cloud work environments. Grounded in `mewcomputer/mew` @ main
and the prior specs (ADE plan, jj spec, misc spec, process-split discussion).

---

## North star & non-negotiables

1. **Accounts are optional and thin.** Every feature in this doc is additive.
   mew must start, run, and be fully useful with zero account, zero network
   calls to mew infrastructure. No phone-home, no nag. The account's pitch is
   "multi-device becomes effortless," never "required."
2. **The server never holds control-granting secrets in usable form.**
   Device private keys never leave devices. User secrets sync only as
   ciphertext the server cannot decrypt. A full server compromise yields:
   public keys, encrypted blobs, connection metadata. Not shells.
3. **Self-hostable.** The account/rendezvous server is one small binary a
   user can run themselves. The hosted instance is a convenience, not a
   moat. Protocol between daemon/client and server is documented like the
   daemon wire protocol is.
4. **Two sync planes, two mechanisms** (the core design decision):
   - **identity plane** — which devices are "you" → *enrollment* (signed
     device-key list). server sees public keys only.
   - **secrets plane** — api keys, mcp creds, config, personas →
     *e2ee vault* (client-side encryption under a passphrase-derived key).
   - ephemeral environments use **neither** for provider creds: they get
     short-lived scoped tokens injected at provision time.

---

## Architecture sketch

```
                      ┌─────────────────────────────┐
                      │  mew-hub (one binary)       │
                      │  - account registry          │
                      │  - device pubkey list (signed)│
                      │  - e2ee vault blob store     │
                      │  - rendezvous / relay        │
                      │  - provisioner API (byo-cloud)│
                      └───────┬─────────────────────┘
              register/heartbeat│        │ enroll, vault get/put
        ┌───────────────────────┤        ├──────────────────────┐
        │                       │        │                      │
  daemon @ homelab        daemon @ vm   web ui / tui client   phone (approver)
  (device key A)          (device key C, (device key B)
                           via enroll token)
```

- clients discover daemons through the hub, then connect **directly** where
  possible (lan, tailscale, direct tcp). the hub relays only as fallback.
  candidate transport: iroh (you already have keys in the picture) — its
  hole-punching + relay model matches this exactly, with device keys as
  iroh node ids. decision point below.
- the hub is stateless-ish: account records, device lists, vault blobs,
  last-seen. no session content, no message history, no file contents ever
  transit it except as opaque relay frames (which are e2e encrypted between
  device keys anyway).

---

## Phase 0 — prerequisites (shared with process-split work)

these are the same enablers identified for process-per-session; do them once.

1. **message-shaped permissions**: replace `oneshot::Sender` in
   `AgentEvent::PermissionRequest` / `WorkspacePermissionRequest` with
   id-correlated request/resolve messages. remote clients cannot hold a
   channel; this is a hard blocker for any remote access.
2. **authenticated daemon connections**: today the trust model is "you can
   reach the socket." add a connection handshake to the wire protocol:

   ```
   ClientMessage::Hello { device_pubkey, signature-over-challenge, proto_version }
   ServerMessage::Challenge { nonce } / HelloAck { daemon_version, ... }
   ```

   local unix-socket connections may skip auth (config
   `[daemon] local_auth = "none" | "required"`, default none — preserves
   current zero-friction local use). tcp/relay connections always require it.
   daemon keeps its own authorized-keys list (`~/.mew/authorized_devices`),
   which works standalone with **no hub at all** — you can manually add a
   device pubkey the way you'd add an ssh key. the hub, when present, just
   automates distribution of this list.
3. **`Ping`/`Pong { version }`** if not already added via doctor — needed for
   version-skew handling once daemons and clients update independently.

phase 0 ships value alone: authenticated remote daemon access, manually
configured. "mew + hand-copied pubkeys" is the tailscale-tier baseline.

---

## Phase 1 — mew-hub: accounts, enrollment, rendezvous

new repo/crate `mew-hub` (keep it out of the workspace or in — either way,
it shares `mew-protocol`-adjacent types via a small `mew-hub-proto` crate).

### account & device model

```
Account { id, handle, created_at, recovery_pubkey }
Device  { pubkey, name, kind: Daemon|Client, added_at,
          added_by: DevicePubkey, revoked: bool,
          sig: signature by an existing device (or recovery key) over
               (account_id, pubkey, name, kind, added_at) }
```

- the device list is a hash-chained log of signed add/revoke entries; any
  device can verify the whole chain client-side. the hub stores and serves
  it but cannot forge entries (it has no signing keys). this is the ssh-CA /
  key-transparency-lite model.
- **bootstrap**: first device self-signs entry #0 and generates a recovery
  key shown once as words/QR (printed, not stored). recovery key can sign a
  new device if all devices are lost.
- **enrollment flows**:
  - *interactive*: new device displays its pubkey as QR/short code; an
    existing device scans/approves and signs the add entry. (phone-as-
    approver is worth supporting early — it's the everyday flow.)
  - *token*: an existing device mints a single-use, time-boxed enrollment
    token (a signed grant: "the bearer key presented within 10min may be
    added with kind=Daemon, name=X"). this is the vm path — no human in the
    loop at boot.
  - *revoke*: any device signs a revoke entry; hub pushes updated list to
    all daemons, which drop live connections from revoked keys. hostile
    revocations (stolen device) are handled by key priority + a 72h
    recovery window — see "device-list chain" under identity decisions
    below.

### rendezvous & connectivity

- daemons register + heartbeat: `{account, device_pubkey, addrs, nat_info}`.
- clients fetch the daemon list for their account, attempt direct connect,
  fall back to hub relay. relay frames are opaque (already e2e via the
  device-key transport).
- **decision: iroh vs hand-rolled.** recommendation: iroh. it gives node
  identity = ed25519 key (aligns with device keys 1:1), hole punching,
  relays, and QUIC streams; you'd hand-roll a worse version otherwise. cost:
  a chunky dependency and running iroh relays or using n0's. mitigation:
  keep the transport behind a trait in `mew-web-bridge`/daemon so plain
  tcp+tls remains a supported transport for the "no hub, no iroh" mode.
- the existing web bridge grows a mode: instead of only proxying
  localhost→unix-socket, it can connect out to remote daemons from the
  account list. the web ui's session rail gains a daemon picker (rides on
  the ADE plan's workspace grouping — a remote daemon is just another
  workspace group source).

### hub implementation notes

- single binary, sqlite, no background workers except heartbeat expiry.
  axum or similar. target: `docker run mew-hub` self-host in one command.
- hub auth for api calls = signature by an enrolled device key over the
  request. no passwords, no sessions, no oauth in v1. (hosted instance may
  later add email-based account *recovery contact*, never login.)
- abuse/quota only matters for the hosted instance; keep limits config-side.

---

## Phase 2 — e2ee vault (secrets & config sync)

### what syncs

tiered, user-controlled:

- **secrets** (default ON when vault enabled): provider api keys, mcp
  credentials — everything `mew-config` credential resolution touches.
- **config** (default ON): config.toml minus machine-specific bits
  (paths, socket), personas, commands, skills prefs.
- **never**: session history, workspace files, device private keys. history
  sync is a different, much bigger project (crdt territory) — explicitly out
  of scope; note it so nobody scope-creeps it in.

### crypto shape (boring on purpose)

- vault key derived from a user passphrase via argon2id; blobs encrypted
  xchacha20-poly1305 (or age as the envelope format — age is tempting for
  auditability and tooling; decision for implementer, pick one, no custom
  constructions).
- hub stores `{account, blob_id, ciphertext, version, updated_at}`.
  last-write-wins with version check; on conflict, keep both and surface in
  the client ("vault conflict: local vs remote, pick"). secrets change
  rarely; lww is fine.
- devices cache the decrypted vault in the os keychain where available
  (keychain/secret-service/wincred), else an on-disk file encrypted under a
  device-local key, so the passphrase is needed once per device, not per
  boot.
- **headless daemons and the vault**: a headless daemon *may* hold a vault
  token (device-wrapped copy of the vault key) if the user opts in per
  device — "this daemon can read my secrets." default off for daemons
  enrolled via token (i.e. vms), which instead use phase-3 scoped injection.

### integration points

- `mew-config` credential resolution gains a vault source, priority:
  env var > local credentials file > vault. explicit and documented, so
  debugging "which key is it using" stays sane. `mew doctor` prints the
  source per provider (already specced) — extend with `vault`.
- ui: settings → account section: enroll status, device list with revoke,
  vault on/off per tier, passphrase change (re-encrypt + bump version).

---

## Phase 3 — one-click environments (byo-cloud)

### flow

1. user configures a **provider** in settings: fly.io token, hetzner token,
   or "docker host over ssh." (fly machines first — api-driven, per-second
   billing, native suspend/resume.)
2. "new environment": pick repo (or blank), pick size, click.
3. hub provisioner (or the client directly — see decision below) calls the
   cloud api with a cloud-init/dockerfile that:
   - installs mew (pinned version), runs `mew doctor --json` as the health
     gate
   - clones the repo using a short-lived repo token (github app installation
     token or a deploy key minted for this env)
   - starts the daemon with an **enrollment token** baked in → daemon
     generates its key, enrolls, registers with rendezvous
   - receives **scoped provider tokens** (not the user's real keys) via
     provisioning secrets — e.g. an anthropic api key created for this env,
     revoked at teardown. where a provider has no scoped-key api, fall back
     to a hub-side metering proxy (later; v1 can just warn "this provider
     key will be placed on the vm").
4. environment appears in the session rail as a remote daemon; sessions on
   it behave identically (that's the payoff of frontend→daemon).
5. **jj tie-in**: env repos are jj-colocated on clone; change-per-session
   means work on a disposable vm is a pushable change — "pull this env's
   work down" is `jj git fetch` from the env's remote or a direct
   daemon-to-daemon fetch (later).

### lifecycle

- idle detection: no active turns + no attached clients for N minutes →
  suspend (fly) or stop (others). resume on connect attempt via rendezvous
  ("daemon asleep — wake?" → provisioner resumes, client retries).
- teardown: revoke device key, revoke scoped tokens, destroy machine, keep a
  tombstone record (name, repo, total cost) in the account.
- alerts channel (misc spec §2) carries "env idle, suspending" / "env failed
  health check" notifications.

### decision: where does the provisioner live

- **v1: in the client/daemon, not the hub.** the user's cloud token stays on
  their device; the hub only learns "a new daemon enrolled." this keeps the
  hub dumb and keeps you out of the business of custodying cloud creds —
  consistent with non-negotiable #2.
- a hub-side provisioner (for true one-click from any browser) is the hosted
  -product version; defer, and note it changes the hub's threat model
  significantly (it would hold cloud tokens — likely under the vault key via
  a client-authorized grant, which is designable but not v1).

---

## Sequencing & effort

| phase | depends on | size | standalone value |
|---|---|---|---|
| 0. auth'd connections + message-shaped permissions | — (shared w/ process split) | M | remote access w/ manual keys |
| 1. hub: accounts, enrollment, rendezvous | 0 | L | multi-device without key copying |
| 2. e2ee vault | 1 | M | secrets/config follow you |
| 3. one-click envs (byo) | 1 (+2 optional) | L | disposable cloud workspaces |

suggested order 0 → 1 → 3 → 2 if the deploy button is the thing you want
soonest — envs don't need the vault (they use scoped tokens), only
enrollment + rendezvous. do 2 before 3 if daily-driver multi-device matters
more than cloud envs.

## identity & "login" (decided)

there is no classical login. auth to the hub = a signature from an enrolled
device key over each request. what feels like login is device enrollment:

- **fresh browser**: web ui generates a keypair on first visit (webcrypto,
  non-extractable, indexeddb), shows its pubkey as qr/short code; an
  existing device (phone is the everyday approver) scans and signs the add
  entry. subsequent visits: key is present, you're in.
- **browser devices are semi-ephemeral**: auto-named
  (`web · chrome · 2026-07-02`), auto-expired from the device list after
  **7 days unseen** (a signed expiry rule the hub applies and any device
  can verify; re-enrollment is cheap by design).
- **handles are webfinger-resolvable and namespaced by domain**:
  `ryan@hub.example.com` → `/.well-known/webfinger` on that domain → hub
  endpoint + stable account id. each hub governs its own namespace, which
  makes squatting and deletion per-hub policy (tombstone + blob purge,
  specced from day one) and makes self-hosted hubs first-class by
  construction. nothing cryptographic signs over the handle — the device
  chain binds to the account id — so handles can be renamed or migrated
  across hubs without touching keys.
- **oauth2/oidc is deferred, and when it lands it is an enrollment onramp,
  not an auth layer**: on the hosted hub, "sign in with github" may gate
  *issuing an enrollment grant* for a fresh device (the familiar-login feel
  people expect), but every subsequent request is still device-key signed
  and the idp is never in the request path. an idp outage or account loss
  can cost you the onramp, never access to already-enrolled devices.

### device-list chain: ordering and hostile-revocation (decided)

- **ordering**: the hub serializes writes to the chain (single writer per
  account). the hub is trusted for *ordering only* — every entry is signed
  by a device key and the full chain verifies client-side, so the hub can
  sequence but never forge. no crdt merge machinery.
- **revocation races**: adopt the atproto did:plc model rather than
  inventing one — **key priority + recovery window**:
  - keys have a priority order; the recovery key outranks all device keys.
  - a higher-priority key may, within a **72h window**, rewrite the chain
    from a recent point to undo lower-priority operations — i.e. if a
    stolen device revokes your devices, your recovery key (or an
    outranking device) forks the chain back to before the hostile entries
    and the hub adopts the higher-priority fork.
  - after the window, entries are final. hubs notify all devices on any
    add/revoke so hostile changes are noticed inside the window (rides the
    alerts channel).
  - borrow the *mechanism*, not the stack: no did:plc dependency, no
    lexicons, no plc.directory — our chain, plc's fork-resolution rule.

### one master secret (decided)

single recovery phrase per account. it is the root of an hkdf tree:

```
master (bip39-style phrase)
 ├─ hkdf "mew/recovery-sign" → recovery signing key (device-chain recovery)
 └─ hkdf "mew/vault"         → vault key (secrets encryption)
```

tradeoff accepted: the phrase now both recovers the account *and* decrypts
secrets, so it is maximally sensitive — docs and ui must say "this phrase
is everything" loudly, and it is never entered anywhere except recovery and
new-vault-device flows. benefit: exactly one thing to keep. domain-separated
derivation means a future split (rotate vault key without touching
recovery) stays possible.

### wire/format decisions

- **transport**: iroh. node id = device pubkey; accept-time check against
  the account device list replaces the explicit challenge dance on iroh
  connections (keep hello/challenge for plain tcp+tls, which lacks free
  peer identity). node ids are treated like ssh pubkeys: fine to display,
  fine in qr codes, never a secrecy boundary. **public discovery (dns /
  pkarr / dht) is disabled** — the hub is the only resolver, so reachability
  info never leaves the account.
- **hub api**: plain http+json, request/response shaped; ws (or iroh
  stream) only for the relay path and change-push. web-first; the tui needs
  only phase-0 auth to connect remotely.
- **enrollment qr / code format**: versioned, prefix `mew` + 3-digit
  version — `mew001:` — followed by a compact payload of
  `{handle, hub endpoint, grant}`. parsers reject unknown versions loudly.
  same prefix scheme reserved for any future scannable artifacts (recovery
  phrase qr, device pubkey display).
- **protocol versioning**: every handshake (client↔daemon, device↔hub)
  carries `proto_version`; each binary advertises
  `min_supported..=current`. policy: a released version must interoperate
  with versions released in the previous **90 days**; incompatible bumps
  require a deprecation release in between. `mew doctor` warns on skew.
- **hosted relay economics**: per-account soft caps on relay bandwidth from
  day one (config-side numbers, generous defaults, non-hyperscaler
  hosting). direct/holepunched connections are uncapped — caps apply only
  to hub-relayed fallback traffic. self-hosted hubs set their own or none.

## what this must never become

a checklist to re-read before each phase ships:

- daemon starts and works with `[account] enabled = false` (the default).
- no telemetry through the hub; heartbeats carry connectivity info only.
- every hub api is speakable by a self-hosted instance; hosted has no
  private endpoints that make self-hosting second-class.
- the words "log in to continue" never appear in mew.
