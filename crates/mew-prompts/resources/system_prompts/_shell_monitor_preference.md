{%- if has_tool("shell_background") or has_tool("bash") %}
{%- if has_tool("shell_monitor") %}

## Waiting For Readiness

When you need to wait for a command condition to become true, use `shell_monitor` with `max_wait_seconds` instead of adding arbitrary `sleep` commands or polling loops inside `bash`. Use `shell_monitor` only for idempotent readiness probes; use `bash` for commands that should run exactly once.
{%- endif %}
{%- endif %}
