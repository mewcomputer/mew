---
title: Extension System Internals
description: Architecture of the extension broker, capability model, sandbox, consent, and token management.
---

`mew-ext-broker` implements extension manifests, capability consent, macOS
Seatbelt profiles, and attach tokens, paired with `mew-hooks-runtime` for
subprocess JSON-RPC transport.

## Crate structure

```text
mew-ext-broker
├── audit.rs                   GateOutcome, GateAuditEntry
├── audit_log.rs               JSONL audit log writer
├── broker.rs                  ExtensionBroker: Dispatcher impl, spawns extensions
├── capabilities.rs            Capability enum, CapabilitySet, CapabilityDelta
├── capability_descriptions.rs Consent prompt builders
├── consent.rs                 ConsentState, ConsentDecision, ConsentResolver
├── discovery.rs               discover_extensions, DiscoveredExtension
├── event_queue.rs             Bounded event queue scaffolding (unwired)
├── manifest.rs                ExtensionManifest, parse_manifest, validate_manifest
├── principal.rs               Principal, PrincipalKind
├── sandbox.rs                 Seatbelt profile generation, SandboxConfig
└── tokens.rs                  Token minting, validation, rotation (keyring + file)
```

## Manifest format

`mew-ext.toml` at the package root:

```toml
[extension]
name = "my-ext"
version = "0.1.0"
description = "An example extension"

[extension.entry]
run = ["node", "dist/index.js"]     # optional; declarative-only if absent

[extension.capabilities.hooks]
observe = true
gate = ["bash"]                     # gate specific tools
gate_mutate = false                 # gate with input mutation
mutate_headers = false              # rewrite provider request headers

[sandbox]
net = false                         # deny network by default
fs_read = ["/etc/hosts"]            # exact additional read paths
fs_write = ["/tmp/output"]          # exact additional write paths

[provides]                          # optional paths relative to package root
skills = "skills/"
commands = "commands/"
personas = "personas/"
subagents = "agents/"
mcp = "mcp.json"
```

### Validation

`validate_manifest` in `manifest.rs` enforces:

- Non-empty `extension.name` and `extension.version`.
- Single-segment `extension.name` (forbids `/`, `\`, and `..` to prevent path
  traversal during install).
- No sensitive paths (`~/.ssh`, `~/.aws`, `~/.gnupg`, or Mew credentials) in
  `sandbox.fs_read` or `sandbox.fs_write`.

## Capability model

Capabilities in `capabilities.rs` map to consent tiers:

| Tier | Capabilities | Consent behavior |
|------|--------------|------------------|
| Always granted | `Storage`, `ConfigRead` | No prompt |
| Low | `Ui`, `Register`, `HooksObserve`, `Events{session,meta}` | Batch prompt |
| Medium | `SessionsRead`, `SessionsManage`, `SessionsPrompt`, `HooksMutate`, `Events{session,full}`, `Events{global,meta}` | Batch prompt |
| High | `HooksGate`, `HooksMutateHeaders`, `HooksMutateShellEnv`, `HooksMutateChatParams`, `PermissionsResolve`, `Events{global,full}` | Individual prompt |
| Highest | `HooksGateMutate` | Individual prompt |

### CapabilitySet

`CapabilitySet` wraps `HashSet<Capability>` with four core operations:

- `satisfies(&Capability) -> bool`: evaluates capability hierarchy (for example,
  `HooksGateMutate` satisfies `HooksGate` and `HooksMutate`;
  `Events{global,full}` satisfies all event scopes).
- `difference(&CapabilitySet) -> CapabilityDelta`: computes new capabilities on
  manifest upgrades.
- `intersect(&CapabilitySet) -> CapabilitySet`: clamps persisted consent to the
  manifest's current requests.
- `legacy_full()` and `observe_only()`: preset sets for bare plugins and
  declined/non-interactive runs.

### Consent flow

`build_consent_resolver` (`crates/mew/src/setup/agent.rs`) runs per extension
at broker spawn time:

1. Load persisted consent state (`extensions/consent.json` in Mew's platform
   data directory; `LEGACY_FULL_SENTINEL` marks full bare-plugin consent).
2. If no prior record exists, prompt for all requested capabilities (batched for
   low/medium tiers, individual for high/highest tiers).
3. If prior consent exists and matches the manifest, clamp persisted grants via
   `intersect`.
4. If the manifest added new capabilities, prompt only for the delta.

### ConsentDecision mapping

```text
Approved         -> legacy_full()    (bare plugins)
Restricted       -> observe_only()   (declined)
ApprovedWithCaps -> granted set      (manifest extensions)
```

`to_caps(fallback)` falls back to `observe_only()` on manifest extensions so a
bare `Approved` decision fails closed.

## Sandbox

### Profile generation

`build_sandbox_profile` in `sandbox.rs` builds a default-deny macOS Seatbelt
S-expression profile allowing:

- Package dir: `(allow file-read* file-write* file-read-metadata (subpath (param "PACKAGE_DIR")))`
- Storage dir: `(allow file-read* file-write* file-read-metadata (subpath (param "STORAGE_DIR")))`
- Widened paths: exact literal rules `(allow file-read* file-read-metadata (literal "/path"))` (plus `file-write*` for `fs_write`)
- Network: emits `(deny network*)` when `sandbox.net` is `false` (note: currently omits an explicit `(allow network*)` rule when `sandbox.net = true` under `(deny default)`)
- Process execution: allowed so the extension process and subprocesses can run
- Pipe I/O: `(allow file-read-data file-write-data)` for open stdin/stdout FDs
  without granting open() on arbitrary paths
- `/dev` and system services: `sysctl-read`, `mach-lookup`,
  `process-info-pidinfo`, and `signal (target self)`

Never add an unfiltered `(allow file-read* file-write*)` rule, which would
bypass the sandbox. `file-read-data` and `file-write-data` restrict syscalls to
already-open descriptors.

### Profile safety and SpawnSpec

`escape_path` escapes backslashes, double quotes, newlines, carriage returns, and
tabs in manifest paths. Because the profile passes through `sandbox-exec -p`,
profiles exceeding 100 KB log a warning against hitting `ARG_MAX` (`E2BIG`).

`SpawnSpec::Command` (`crates/mew-hooks-runtime/src/transport.rs`) carries an
optional `sandbox: Option<(String, Vec<(String, String)>)>` tuple so `to_command()`
wraps the launch with `sandbox-exec -p <profile> -D KEY=VALUE ...`. Bare plugins
use `SpawnSpec::Path` without a sandbox.

## Token management

`tokens.rs` generates and rotates attach tokens:

- **Token format**: 26-character ULID (`ulid::Ulid::new()`).
- **Storage**: keyring (`mew-ext-tokens` service) with a `0600` fallback file
  in `mew_config::config_dir().join("extensions").join("tokens")` (`<name>.token`,
  directory mode `0700`). Keyring-backed tokens also write a marker file so
  `list_tokened_extensions` can find them during rotation.
- **Validation and rotation**: `constant_time_eq` avoids timing leaks;
  `rotate_all_tokens` rotates every tokened extension and returns per-extension
  results so one failure does not abort the rest.

:::note[Unwired token enforcement]
`validate_token` and `State.revoked_extensions` do not yet have daemon callers;
socket-attach verification depends on moving broker ownership into the daemon.
:::

## Discovery

`discover_extensions(cwd)` in `discovery.rs` scans:

1. Project-local: `<cwd>/.mew/extensions/<name>/`
2. Global: `~/.config/mew/extensions/<name>/` (via `directories::UserDirs` so
   macOS matches the CLI config directory)

Project packages override global packages on duplicate names. `DiscoveredExtension`
exposes `provides_skills()`, `provides_personas()`, and `provides_subagents()`,
each returning `Option<PathBuf>` for the respective loaders (`mew-skills`,
`mew-personas`, `mew-subagents`).

## Broker spawn flow

`ExtensionBroker::from_dirs_filtered_with_config` in `broker.rs`:

1. **Bare plugins**: spawn via `SpawnSpec::Path` without a sandbox and invoke
   the consent resolver with `None` (`Approved` maps to `legacy_full()`).
2. **Manifest extensions**: skip disabled or declarative-only packages (no
   `entry.run`), generate a Seatbelt profile when `sandbox_available()` is
   true, spawn via `SpawnSpec::Command`, and resolve consent with
   `Some(&manifest)` (missing resolver defaults to `ApprovedWithCaps` for tests).

Both paths sort `(Arc<PluginSlot>, Principal)` pairs alphabetically by slot name
for deterministic hook execution order.

## State integration

`mew_config::State.revoked_extensions` records revoked extension names; `mew ext
revoke` appends to the list and `mew ext rotate-all` clears it.

## Platform support

| Platform | Sandbox | Status |
|----------|---------|--------|
| macOS | Seatbelt via `sandbox-exec` | Implemented |
| Linux | Landlock + seccomp | Deferred |
| Windows | None | Unplanned |

On platforms where `sandbox_available()` is `false`, the broker logs a warning,
runs extensions unsandboxed, and `mew ext doctor` reports `[unsandboxed (platform)]`.
