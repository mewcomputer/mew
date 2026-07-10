---
title: Extension System Internals
description: Architecture of the extension broker, capability model, sandbox, consent, and token management.
---

The extension system is implemented in the `mew-ext-broker` crate, with
transport support from `mew-hooks-runtime`. This document covers the
internal architecture for contributors extending or modifying the
extension system.

## Crate structure

```
mew-ext-broker
├── broker.rs              — ExtensionBroker: Dispatcher impl, spawns extensions
├── capabilities.rs        — Capability enum, CapabilitySet, CapabilityDelta
├── capability_descriptions.rs — Consent prompt builders
├── consent.rs             — ConsentState, ConsentDecision, ConsentResolver
├── discovery.rs           — discover_extensions, DiscoveredExtension
├── manifest.rs            — ExtensionManifest, parse_manifest, validate_manifest
├── sandbox.rs             — Seatbelt profile generation, SandboxConfig
├── tokens.rs              — Token minting, validation, rotation (keyring + file)
├── audit_log.rs           — Gate audit logging
├── event_queue.rs         — Bounded event queue scaffolding (not yet wired)
└── principal.rs           — Principal, PrincipalKind
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

[extension.sandbox]
net = false                         # deny network by default
fs_read = ["/etc/hosts"]            # additional read paths
fs_write = ["/tmp/output"]          # additional write paths

[provides]                          # optional, relative to package root
skills = "skills/"
personas = "personas/"
subagents = "agents/"
```

### Validation

`validate_manifest` in `manifest.rs` checks:
- Non-empty name and version
- Name is a single path component (no `/`, `\`, or `..` — prevents path
  traversal during install)
- No sensitive paths in `sandbox.fs_read`/`fs_write` (denies `~/.ssh`,
  `~/.aws`, `~/.gnupg`, credentials files)

## Capability model

Capabilities are defined in `capabilities.rs`. The `Capability` enum has
~17 variants with risk tiers:

| Tier | Capabilities | Consent behavior |
|------|-------------|-----------------|
| Always granted | `Storage`, `ConfigRead` | No prompt |
| Low | `Ui`, `Register`, `HooksObserve`, `Events{session,meta}` | Batch prompt |
| Medium | `SessionsRead`, `SessionsManage`, `SessionsPrompt`, `HooksMutate`, `Events{session,full}`, `Events{global,meta}` | Batch prompt |
| High | `HooksGate`, `HooksMutateHeaders`, `HooksMutateShellEnv`, `HooksMutateChatParams`, `PermissionsResolve`, `Events{global,full}` | Individual prompt |
| Highest | `HooksGateMutate` | Individual prompt |

### CapabilitySet

A `HashSet<Capability>` wrapper with:
- `satisfies(&Capability) -> bool` — checks with hierarchy (e.g.,
  `HooksGateMutate` satisfies `HooksGate` and `HooksMutate`;
  `Events{global,full}` satisfies all event scopes)
- `difference(&CapabilitySet) -> CapabilityDelta` — for consent delta
  detection on manifest upgrades
- `intersect(&CapabilitySet) -> CapabilitySet` — for clamping persisted
  consent to manifest's requested set
- `legacy_full()` — all capabilities (for bare plugins)
- `observe_only()` — storage + config read + hooks observe (for
  restricted/declined)

### Consent flow

The consent resolver (`build_consent_resolver` in `mew/src/setup/agent.rs`)
runs per extension at broker spawn time:

1. Check persisted consent state (`consent.json`)
2. If no prior consent: prompt for all requested capabilities (two-phase:
   batch non-sensitive, then individual sensitive)
3. If prior consent exists and manifest unchanged: clamp persisted caps
   to manifest's requested set via `intersect`
4. If manifest grew (new capabilities): prompt only for the delta

Consent state persists capability ID strings (not the enum) to
`~/.local/share/mew/extensions/consent.json`. The `LEGACY_FULL_SENTINEL`
string marks bare-plugin full-access consent.

### ConsentDecision → CapabilitySet mapping

```
Approved         → legacy_full()    (bare plugins)
Restricted       → observe_only()   (declined)
ApprovedWithCaps → the granted set  (manifest extensions)
```

The `to_caps(fallback)` method maps the decision, using `fallback` only
for the `Approved` variant. For manifest extensions, `observe_only()` is
passed as the fallback (fail-closed if somehow Approved reaches the
manifest path).

## Sandbox

### Profile generation

`build_sandbox_profile` in `sandbox.rs` generates a Seatbelt
S-expression profile. The profile is default-deny with allow rules for:

- Package dir (read/write): `(allow file-read* file-write* (subpath (param "PACKAGE_DIR")))`
- Storage dir (read/write): `(allow file-read* file-write* (subpath (param "STORAGE_DIR")))`
- Widened paths from manifest: `(allow file-read* (literal "/path"))`
- Network: denied unless `sandbox.net = true`
- Process exec/fork: allowed (extension needs to run)
- Pipe I/O: `(allow file-read-data file-write-data)` — allows read/write
  on already-open FDs (stdin/stdout pipes) without allowing open() on
  arbitrary paths
- `/dev` access: for `/dev/null` etc.
- System services: `sysctl-read`, `mach-lookup`, `process-info-pidinfo`,
  `signal (target self)`

**Security note:** The profile must NOT contain a blanket
`(allow file-read* file-write*)` without a path filter — that would
negate the sandbox entirely. The `file-read-data`/`file-write-data` rule
is safe because it covers read()/write() syscalls on open FDs, not
open() on new paths.

### Profile injection prevention

`escape_path` escapes backslash, double-quote, newline, carriage return,
and tab in path literals to prevent Seatbelt profile injection via
crafted manifest paths.

### ARG_MAX guard

The profile text is passed as a CLI argument to `sandbox-exec -p`. If
the profile exceeds 100KB (many widened paths), a warning is logged. The
extension may fail to spawn with `E2BIG`.

### SpawnSpec

`SpawnSpec::Command` in `mew-hooks-runtime/src/transport.rs` carries an
optional `sandbox: Option<(String, Vec<(String, String)>)>` — a tuple of
(profile_text, params) kept as plain types to avoid a cross-crate
dependency. `to_command()` wraps the extension command with
`sandbox-exec -p <profile> -D KEY=VALUE ...`.

Bare plugins use `SpawnSpec::Path` — no sandbox.

## Token management

`tokens.rs` implements attach tokens for socket-transport extensions:

- **Minting**: `ulid::Ulid::new()` generates 26-character ULIDs (128
  bits of entropy)
- **Storage**: keyring first (`mew-ext-tokens` service), file fallback at
  `~/.config/mew/extensions/tokens/<name>.token` (permissions 0600, parent
  dir 0700)
- **Marker files**: when a token is stored in the keyring, an empty
  marker file is written so `list_tokened_extensions` can discover it
  for rotation
- **Validation**: `constant_time_eq` prevents timing side-channels
- **Rotation**: `rotate_all_tokens` collects per-extension results
  instead of aborting on first failure (partial success is reported)

> **Not yet wired:** `validate_token` is exported but has no callers. The
> daemon socket-attach path that would consume it requires the daemon
> to own a broker (currently constructed per-session). This is deferred
> to a separate plan.

## Discovery

`discover_extensions(cwd)` in `discovery.rs` scans:
1. Project-local: `<cwd>/.mew/extensions/<name>/`
2. Global: `~/.config/mew/extensions/<name>/`

`discover_extensions_from_dirs(project_dir, global_dir)` is the
testable variant that takes explicit paths. The production function
delegates to it using `directories::UserDirs` (not `ProjectDirs` —
important: `ProjectDirs` resolves to a different path on macOS).

Dedup precedence: project beats global on name collision.

`DiscoveredExtension` carries the parsed manifest, root path, scope,
and `[provides]` paths. `provides_skills()`, `provides_personas()`,
`provides_subagents()` extract individual path lists that feed into the
existing loaders (`mew-skills`, `mew-personas`, `mew-subagents`).

## Broker spawn flow

`ExtensionBroker::from_dirs_filtered_with_config` in `broker.rs`:

1. **Bare plugins** (legacy path): spawn via `SpawnSpec::Path`, no
   sandbox. Consent resolver called with `None` manifest. `Approved` →
   `legacy_full()`.

2. **Manifest extensions**: skip if disabled or declarative-only (no
   `entry.run`). Build sandbox profile if `sandbox_available()`. Spawn
   via `SpawnSpec::Command` with sandbox. Consent resolver called with
   `Some(&manifest)`. `ApprovedWithCaps` → granted set. No resolver →
   `ApprovedWithCaps(requested_capabilities())` (fail-open for testing).

Both paths produce `(Arc<PluginSlot>, Principal)` pairs sorted
alphabetically by slot name for deterministic hook ordering.

## State integration

`mew-config::State` has a `revoked_extensions: Vec<String>` field
(serde default, skipped when empty). `mew ext revoke` adds to this list;
`mew ext rotate-all` clears it. The daemon will check this list on every
socket attach when the attach path ships.

## Platform support

| Platform | Sandbox | Status |
|----------|---------|--------|
| macOS | Seatbelt via `sandbox-exec` | Shipped |
| Linux | Landlock + seccomp | Deferred |
| Windows | None | Not planned |

On unsupported platforms, `sandbox_available()` returns `false`, the
broker logs a warning, and the extension runs unsandboxed. `mew ext
doctor` shows `[unsandboxed (platform)]`.
