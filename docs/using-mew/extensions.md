---
title: Extensions
description: Install, manage, and troubleshoot Mew extensions.
---

An extension package is a directory containing a `mew-ext.toml` manifest and an
optional entry program.

Currently, `mew run` loads personas, skills, and subagents from declarative
`[provides]` paths. Daemon-backed chat loads project and global markdown
definitions directly and skips extension packages. Neither command launches
extension processes for hooks or dynamic tools. The sandboxing and consent
sections below describe the experimental extension broker runtime.

## Installing extensions

### From a git URL

```sh
mew ext install https://github.com/user/my-extension.git
```

Shallow-clones the repository and copies it into `~/.config/mew/extensions/`.

### From a local path

```sh
mew ext install ./my-extension
```

The CLI currently has a bug when installing from a local directory path (`mew ext
install` copies an empty temporary directory). Until that is fixed, copy local
extension directories directly into `~/.config/mew/extensions/<name>/` or
`.mew/extensions/<name>/`.

### Flags

| Flag | Description |
|------|-------------|
| `--name <name>` | Override the install name (defaults to the manifest or directory name) |
| `--force` | Overwrite an existing installation |
| `--dry-run` | Print what would be installed without modifying files |

`--dry-run` prints the derived name, destination path, and conflict status. For
git URLs, it derives the name from the URL without cloning.

### Install directories

Global extensions live in `~/.config/mew/extensions/<name>/`. Place project-local
extensions manually in `.mew/extensions/<name>/` (`mew ext install` only targets
the global directory).

## Managing extensions

### List installed extensions

```sh
mew ext list
```

Prints each discovered extension package with its name, version, scope, status,
and capabilities, followed by any bare plugins and their status. Run `mew ext
doctor` to inspect source paths.

### Enable or disable

```sh
mew ext enable <name>
mew ext disable <name>
```

Records whether an extension is enabled in `state.toml`. Both `mew ext list` and
`mew ext doctor` display that status.

### Remove

```sh
mew ext remove <name>
```

Deletes an extension package from disk. Delete bare plugins from
`~/.config/mew/plugins/` or `.mew/plugins/` manually.

### Doctor

```sh
mew ext doctor
```

Checks extension discovery, conflicts, and runtime readiness:

- Discovery paths (global and project-local)
- Package version, scope, status, entry type, and source path
- Sandbox status per extension:
  - `[sandboxed]`: eligible for the macOS Seatbelt sandbox
  - `[unsandboxed (platform)]`: no OS sandbox on Linux or Windows
  - `[unsandboxed (legacy)]`: bare plugin without a sandbox
  - `[n/a]`: declarative-only package with no entry process
- Name conflicts across discovery directories

## Sandboxing

On macOS, extensions with an entry program run inside a Seatbelt sandbox:

- **Filesystem**: read/write access limited to the extension's package directory
  and its per-extension storage directory
  (`~/.config/mew/extensions/storage/<name>/`).
- **Network**: manifest declares `sandbox.net = true` to request network access
  (currently omitted in generated Seatbelt rules, so network stays blocked under
  the default deny profile).
- **Process execution**: allowed so the entry executable can run.

The manifest `[sandbox]` table can declare exact extra paths (`fs_read` for
read-only paths and `fs_write` for read/write paths), which appear in the first
consent prompt. Seatbelt compiles these as literal path rules, and sensitive
paths (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.config/mew/credentials.json`) remain
blocked even when listed.

Linux and Windows have no OS sandbox, so `mew ext doctor` marks executable
packages on those platforms as `[unsandboxed (platform)]`.

## Consent

When the experimental broker loads a new extension or detects new capabilities
after an update, it prompts for capability consent:

- **Always granted** (`storage`, `config:read`): granted automatically without
  prompting.
- **Batched** (`ui`, `register`, `hooks:observe`, `sessions:*`, `hooks:mutate`,
  `events:session:*`, `events:global:meta`): approved together in a single
  yes/no prompt.
- **Individual** (`hooks:gate`, `hooks:gate:mutate`, `hooks:mutate:headers`,
  `hooks:mutate:shell_env`, `hooks:mutate:chat_params`, `permissions:resolve`,
  `events:global:full`): prompted one capability at a time.

Mew writes consent decisions to `extensions/consent.json` in the platform data
directory (`~/Library/Application Support/ai.mew.mew/extensions/consent.json` on
macOS, `~/.local/share/mew/extensions/consent.json` on Linux). Delete an
extension's entry from that file to prompt again on next load.

In non-interactive runs, new extensions are restricted to observe-only
capabilities while existing consent decisions remain active.

## Attach tokens

The CLI prints, revokes, and rotates attach tokens for a planned extension
attach flow; the daemon does not validate extension attach tokens yet.

### Show a token

```sh
mew ext token <name>
```

Prints the token to stdout and prints a warning to stderr when stdout is a
terminal. Pipe to a clipboard utility to keep tokens out of scrollback:

```sh
mew ext token my-ext | pbcopy
```

### Revoke a token

```sh
mew ext revoke <name>
```

Deletes the stored token and adds the extension name to `revoked_extensions` in
`state.toml`.

### Rotate all tokens

```sh
mew ext rotate-all
```

Rotates every existing `.token` file in the token store and clears the revoked
list. Use this after rotating credentials or if a token may have leaked.

Tokens use the system keyring when available, falling back to `0600` files under
`~/.config/mew/extensions/tokens/<name>.token`.

## Extension packages vs. bare plugins

| Feature | Extension packages | Bare plugins |
|---------|-------------------|--------------|
| Manifest | `mew-ext.toml` required | None |
| Discovery | `extensions/` directory | `plugins/` directory |
| Sandbox | Yes (macOS) | No |
| Consent | Capability-based prompts | Legacy full-access prompt |
| Install via CLI | `mew ext install` (git URLs) | Manual file copy |
| Status in doctor | `[sandboxed]` or `[unsandboxed (platform)]` | `[unsandboxed (legacy)]` |

Bare plugins are standalone executables in `~/.config/mew/plugins/` or
`.mew/plugins/`. The experimental runtime launches them without an OS sandbox
and prompts for full access on first load.
