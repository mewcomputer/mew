# Progress — 2026-06-21

## Personas v2 polish (all committed)

- **Sidebar section**: collapsible "Personas" section between Tools and MCP, active persona marked in purple
- **Confirm modal**: `/persona <name>` shows a diff (model, tools, deny, skills changes) before applying. Confirm/Cancel buttons, y/n shortcuts
- **switch_persona tool**: model-callable, queues switch at end-of-turn (not mid-turn), gated by PermissionEngine (Mutating sensitivity)
- **tools_deny**: PersonaConfig extended with denylist. Applied after allowlist in turn.rs tool filter
- **skills allowlist**: PersonaConfig gains `skills` field. Skill tool gates by filter, system prompt skills XML rebuilt on persona change
- **Minijinja templating**: `template: true` in frontmatter renders persona body with `supports_vision`, `tools`, `denied_tools`, `persona_name`

## Secrets (all committed)

- **Config**: `secrets.files.paths` + `secrets.words.values` in config.toml
- **Permission pre-check**: `read` of secret-file glob forces Prompt unless literal allow rule
- **Redaction**: `secrets.rs` module with `redact_secret_words()` — replaces values with `[REDACTED]`, preserves structure
- **Wired into**: Read, Bash (before truncation), Grep (drops secret-file lines + redacts words), Glob (drops secret-file results)

## Shell command decomposition (all committed)

- **`mew-config/src/shell.rs`**: splits compound commands on `|`, `&&`, `;`, respects quotes
- **Opaque detection**: `$(...)`, backticks, `<(...)`, `eval`, `bash -c`, `| sh`, `| xargs` → force Prompt
- **PermissionEngine**: compound commands require all programs allowed; single commands use prefix matching (backward compat)
- **MatchConditions**: `command_program` + `command_subcommand` fields added

## Hooks runtime overhaul (all committed)

- **21 hooks** in Dispatcher trait (was ~12, many unwired)
- **HookId enum**: single source of truth (as_wire, as_config, ALL, Display)
- **PluginHookConfig**: per-plugin `disabled_hooks`, `matchers`, `timeout_ms`
- **on_register_tools**: subprocess plugins can now register async tools (ToolRegistration::execute is now async)
- **Parallel dispatch**: `pipe_json_filtered` uses `join_all` (latency = max, not sum)
- **Plugin health**: `healthy: AtomicBool`, writer drain on crash, user notification
- **config_read**: wired to PluginInfo (session_id, model, provider, workspace, active_persona)
- **ChatParams/headers**: flow through to provider Request, OpenAI adapter uses them
- **New hooks**: on_user_input, on_persona_change, on_session_save, on_model_finish, on_provider_event, on_tool_error, on_subagent_start/end, on_pre_model_turn, on_stop, on_pre/post_compaction

## Telemetry

- **telemetry-exporter.rs** example plugin: Prometheus /metrics endpoint on :9090
- Collects token/cost/tool/turn metrics from hooks
- Zero external deps (std::net only)

## Decisions made

- `on_event(&dyn Any)` removed entirely — replaced with specific typed hooks (was !Send, couldn't forward to subprocess)
- `pipe_json_filtered` changed from sequential (pipe) to parallel (broadcast, last alphabetical wins) — documented as behavior change
- Plugin restart deferred — needs Arc<Mutex<Option<PluginProcess>>> per slot (documented as TODO)
- ProviderEvent derives Serialize/Deserialize (field: &'static str limitation noted for Rust in-process plugins)

## Next up

- Jobs (async subagents + background shell) — the next feature milestone
