## Waiting For Readiness

When a readiness-monitor tool is available, use it with its documented wait
limit instead of adding arbitrary sleeps or polling loops inside a shell
command. Use readiness monitoring only for idempotent probes; use the command
tool for work that should run exactly once.
