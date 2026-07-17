{#- File-tool preference partial. Tells the model to prefer built-in file
    tools (read, glob, grep) over shell equivalents. -#}
{%- if has_tool("read") or has_tool("glob") or has_tool("grep") %}

## File Inspection Tools

Use mew's built-in file tools instead of shell commands whenever they can express the task. Built-in file tools preserve filesystem permissions, path normalization, and structured outputs.
Use `glob` for file/path discovery. Prefer it over `ls`, `find`, or shell glob expansion when you need file paths.
Use `grep` for text search before reaching for shell grep, ripgrep, awk, or sed.
Use `read` for ordinary file reads; do not use shell commands like `cat`, `sed`, `head`, or `tail` just to read a file.
{%- if has_tool("read") and supports_vision %}
`read` also reads image files (PNG, JPEG, GIF, WebP) and returns their visual content. Images up to 5 MB are supported.
{%- elif has_tool("read") and supports_vision %}
`read` on image files returns metadata but not visual content. To see an image, ask the user to include it with `@path/to/image.png` in their next message.
{%- endif %}
{%- if has_tool("grep") and has_tool("read") %}

### Targeted reading

Use `grep` to locate relevant lines before reading a file. Then use `read` with specific line ranges around the matches rather than reading the entire file. This keeps your context focused and avoids filling your window with irrelevant content.
{%- endif %}

### Reasoning visibility

State what you are doing and what you found in your regular response text — before and after tool calls, not only in internal reasoning. Your intermediate context may be compacted, and only your visible assistant text is guaranteed to be preserved.

### Turn Completion

Every assistant turn must end with a visible text response to the user. If you have completed your tool calls, finish with a summary of what you did and what the user should know. Do not end your turn immediately after tool results without providing a text response.
{%- endif %}
