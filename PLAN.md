# plan: edit model size tiers (nano/micro/deci) from the web frontend

"model size config" = the router tiers. `[providers.<id>]` maps `nano` (smallest),
`micro` (general), and `deci` (most capable) to `provider/model` ids; subagents
pin a tier with `model: nano|micro|deci`, and the permission classifier uses
`micro`. the protocol exposes none of this today.

## decisions

- `config.toml` is the single source of truth, as for `[web_search]`; the ui
  writes it with the existing format-preserving `atomic_write`.
- only the **designated router provider** is editable: `find_router_provider`
  prefers one literally named `router`, else the first with `kind = "router"`.
  direct providers are left alone. legacy `small`/`big` fields are untouched.
- each tier is written as a full `provider/model` id; an empty value removes
  the key (so `nano` falls back to `micro`, per docs). values are validated for
  the `provider/model` shape before writing. no credential fields in the ui.
- three values are sent in one `SetRouterTiers` request (one file write).
- changes are daemon-wide and apply to the next session built; `build_session_agent`
  overrides the tier fields on its per-session `Config` clone before building
  the router/classifier. the ui states this.

## steps

### 1. config: `crates/mew-config/src/lib.rs`
- `pub struct RouterTiers { pub nano: String, pub micro: String, pub deci: String }` with `Default`.
- `pub fn update_router_tiers_at(path, provider_id, &RouterTiers) -> Result<()>`:
  parse `toml_edit::DocumentMut`, ensure `[providers.<id>]` exists, set or
  remove each tier key, atomic-write; refuse an empty provider id.
- tests: preserves comments/other tables; writes all three; empty tier removes
  the key; missing file creates the table; returned error for a bad provider id.

### 2. protocol: `crates/mew-protocol/src/lib.rs`
- `RouterTierInfo { id, model }`, `RouterInfo { provider, tiers: Vec<RouterTierInfo> }`.
- `DaemonSettings.router: Option<RouterInfo>` (`skip_serializing_if`).
- `ClientMessage::SetRouterTiers { provider, nano, micro, deci, request_id }`.
- roundtrip tests; add to control-scope and `DaemonSettingsFailed` lists in the daemon.

### 3. daemon: `session.rs`, `lib.rs`
- `SessionManager.router: watch::Sender<RouterInfo>` seeded from the first
  router provider in `Config`; `pub fn set_router_config(&self, RouterInfo)`;
  `apply_router_tiers(provider, nano, micro, deci, authorization)` with an
  `_at` split for tests, writing config.toml and broadcasting `settings_changed`.
- `AgentBuildParams.router: RouterInfo`; both build sites fill it.
- `daemon_settings_message` includes `router: Some(manager.router_info())`.
- `mew` daemon seed from `cfg` (capture before the builder closure moves it),
  and `commands/run.rs` seeds from `cfg` directly.

### 4. wiring: `crates/mew/src/setup/agent.rs`
- `build_session_agent` takes `router: &RouterInfo`, clones `cfg` into a
  session-local value, and overwrites the target provider's `nano`/`micro`/`deci`
  from the live settings before any router/classifier construction. clear a
  field when its value is empty. update the five call sites and tests.

### 5. web client + ui
- `RouterInfo`/`RouterTierInfo` types, `DaemonSettings.router`, the message
  union entry, `setRouterTiers`, and the emit mapping; client wire test.
- new `components/router-settings.tsx`: a "Model sizes" SettingsGroup showing
  the router provider and three model pickers. options come from
  `availableModels` in the session store (the `ListModels` path); the current
  value is shown even when it is not in the catalog. send one `setRouterTiers`
  call on change; note "applies to new and resumed sessions".
- render in the general section of `settings-panel.tsx`; tests for rendering,
  current values, and a change sending the full request; rebuild `dist`.

### 6. verify
- fmt on touched files, clippy on touched crates, crate tests, tsc both web
  packages, ui suite, arch-check, `CURRENT.md` entry.

## out of scope
- ui editing of credentials, base_url, provider kind, or custom model entries.
- live change to an already-running session (next session only).
- legacy `small`/`big` router aliases.
