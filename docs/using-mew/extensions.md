---
title: Extensions
description: Install, manage, and troubleshoot mew extensions.
---

Extensions are packaged plugins that add tools, slash commands, hooks,
personas, skills, and subagents to mew. Each extension is a directory
containing a `mew-ext.toml` manifest and an optional entry program.

## Installing extensions

### From a git URL

```bash
mew ext install https://github.com/user/my-extension.git
```

Shallow-clones the repo and copies it to `~/.config/mew/extensions/`.

### From a local path

```bash
mew ext install ./my-extension
```

Copies the directory recursively. The source must be a directory (not a
single file).

### Flags

| Flag | Description |
|------|-------------|
| `--name <name>` | Override the install name (defaults to the manifest name or directory name) |
| `--force` | Overwrite if already installed |
| `--dry-run` | Show what would be installed without copying any files |

`--dry-run` shows the name, destination, and conflict status without
modifying anything. For git URLs, dry-run derives the name from the URL
without cloning.

### Where extensions are installed

Extensions install to the global directory:
`~/.config/mew/extensions/<name>/`

Project-local extensions go in `.mew/extensions/<name>/` — place these
manually, not via `mew ext install` (which always installs globally).

## Managing extensions

### List installed extensions

```bash
mew ext list
```

Shows all discovered extension packages and bare plugins with their
status (enabled/disabled) and source path.

### Enable or disable

```bash
mew ext enable <name>
mew ext disable <name>
```

Disabling an extension prevents it from loading on next startup. It
remains installed but is skipped during discovery.

### Remove

```bash
mew ext remove <name>
```

Removes an extension package from disk. Bare plugins (legacy executables
in `~/.config/mew/plugins/`) cannot be removed this way — delete them
manually.

### Doctor

```bash
mew ext doctor
```

Diagnoses extension discovery, conflicts, and health. Shows:

- Discovery paths (global and project-local)
- Each extension with version, scope, status, and entry type
- Sandbox status per extension:
  - `[sandboxed]` — running under macOS Seatbelt sandbox
  - `[unsandboxed (platform)]` — sandbox not available (Linux/Windows)
  - `[unsandboxed (legacy)]` — bare plugin, no sandbox
  - `[n/a]` — declarative-only extension (no process)
- Conflicts (duplicate names across discovery paths)

## Sandboxing

On macOS, extensions with an entry program run inside a Seatbelt sandbox
that restricts what the extension process can do:

- **Filesystem**: read/write limited to the extension's own package
  directory and a per-extension storage directory
  (`~/.config/mew/extensions/storage/<name>/`)
- **Network**: denied by default unless the manifest declares
  `sandbox.net = true`
- **Process execution**: allowed (the extension needs to run)

The manifest can widen filesystem access:
- `fs_read` — additional read-only paths
- `fs_write` — additional read/write paths

These widenings appear in the consent prompt when the extension is first
loaded. Sensitive paths (`~/.ssh`, `~/.aws`, `~/.gnupg`) are always
denied regardless of manifest widenings.

On Linux and Windows, the OS sandbox is not yet available. Extensions
run unsandboxed with a visible warning in `mew ext doctor`. Linux
Landlock + seccomp support is planned for a future release.

## Consent

When an extension is loaded for the first time (or after an upgrade that
adds new capabilities), mew shows a consent prompt listing what the
extension can do:

- **Non-sensitive capabilities** (storage, config read, UI
  notifications, observe hooks) are batched in a single yes/no prompt.
- **Sensitive capabilities** (gate hooks, mutation hooks, header
  rewriting, permission resolution, full event access) are prompted
  individually.

Your choices are persisted to
`~/.local/share/mew/extensions/consent.json` and remembered on subsequent
loads. To re-consent, delete the entry for that extension from the file
and restart.

In non-interactive mode (piped stdin, CI), new extensions are
auto-restricted to observe-only. Existing consent decisions are
respected.

## Attach tokens

Extensions that connect to a running daemon (rather than being spawned
by it) use attach tokens for authentication. Token management:

### Show a token

```bash
mew ext token <name>
```

Prints the token to stdout. When stdout is a terminal, a warning is
printed to stderr first. Pipe to a clipboard tool to avoid the token
appearing in scrollback:

```bash
mew ext token my-ext | pbcopy
```

### Revoke a token

```bash
mew ext revoke <name>
```

Invalidates the token and adds the extension to the revoked list in
`state.toml`. A revoked extension cannot attach to the daemon even with
a valid token.

### Rotate all tokens

```bash
mew ext rotate-all
```

Re-mints all extension tokens and clears the revoked list. Use this if
a token may have been compromised, or after rotating daemon credentials.

Tokens are stored in the system keyring when available, with a file
fallback at `~/.config/mew/extensions/tokens/<name>.token` (permissions
`0600`).

> **Note:** Token validation is not yet wired into the daemon's extension
> attach path. Tokens are minted and managed via CLI, but the daemon does
> not yet check them on attach. This infrastructure is in place for when
> the daemon socket-attach feature ships.

## Slash command aliases

mew accepts both singular and plural forms for some commands:

| Alias | Canonical |
|-------|-----------|
| `/models` | `/model` |
| `/session` | `/sessions` |
| `/permission` | `/permissions` |

## Extension packages vs bare plugins

| Feature | Extension packages | Bare plugins |
|---------|-------------------|--------------|
| Manifest | `mew-ext.toml` required | None |
| Discovery | `extensions/` dir | `plugins/` dir |
| Sandbox | Yes (macOS) | No |
| Consent | Capability-based prompts | Legacy full-access prompt |
| Install via CLI | `mew ext install` | Manual file copy |
| Status in doctor | `[sandboxed]` or `[unsandboxed (platform)]` | `[unsandboxed (legacy)]` |

Bare plugins (executables in `~/.config/mew/plugins/`) are the original
plugin format. They continue to work but are not sandboxed and receive a
legacy full-access consent prompt on first load. A future release will
add deprecation warnings pointing toward the extension package format.
