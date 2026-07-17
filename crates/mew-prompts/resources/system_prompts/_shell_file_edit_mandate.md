{#- Shell file-edit mandate partial. Fires when a shell tool AND a file
    writing/editing tool are both available. mew's tools are: bash,
    edit_hashline, edit_str_replace, write. -#}
{%- set has_shell = has_tool("bash") or has_tool("shell_background") or has_tool("shell_monitor") %}
{%- if has_shell and has_tool("edit_str_replace") %}

## Writing File Content

You MUST use `edit_str_replace` to create, overwrite, or modify file content. Do NOT use heredocs (`<<EOF`), `cat > file`, or pipe-redirect of literal content (`... | tee file` where the input is a literal string emitted by you) to write file content via a shell tool.

Shell tools are for running commands (build, test, git, search) and observing their output. Piping a real command's output to a file is fine — e.g. `make build | tee build.log` to capture log lines. `tee` for observability is allowed; `tee` for content authorship is not. The distinction is *what is being written*: literal content emitted by you goes through `edit_str_replace`; output produced by a command can use any shell redirect.
{%- elif has_shell and has_tool("edit_hashline") %}

## Writing File Content

You MUST use `edit_hashline` to create, overwrite, or modify file content. Do NOT use heredocs (`<<EOF`), `cat > file`, or pipe-redirect of literal content (`... | tee file` where the input is a literal string emitted by you) to write file content via a shell tool.

Shell tools are for running commands (build, test, git, search) and observing their output. Piping a real command's output to a file is fine — e.g. `make build | tee build.log` to capture log lines. `tee` for observability is allowed; `tee` for content authorship is not. The distinction is *what is being written*: literal content emitted by you goes through `edit_hashline`; output produced by a command can use any shell redirect.
{%- elif has_shell and has_tool("write") %}

## Writing File Content

You MUST use `write` to create or overwrite file content. Do NOT use heredocs (`<<EOF`), `cat > file`, or pipe-redirect of literal content (`... | tee file` where the input is a literal string emitted by you) to write file content via a shell tool.

Shell tools are for running commands (build, test, git, search) and observing their output. Piping a real command's output to a file is fine — e.g. `make build | tee build.log` to capture log lines. `tee` for observability is allowed; `tee` for content authorship is not. The distinction is *what is being written*: literal content emitted by you goes through `write`; output produced by a command can use any shell redirect.
{%- endif %}
