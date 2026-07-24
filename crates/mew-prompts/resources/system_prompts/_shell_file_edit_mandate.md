{#- Keep this guidance independent of the concrete editor and shell tool set. -#}

## Writing File Content

When a dedicated file-editing tool is available, use it to create, overwrite,
or modify file content. Do not use heredocs, `cat > file`, or pipe-redirection
of literal content through a shell tool.

Shell tools are for running commands and observing their output. Piping a real
command's output to a file is fine; literal content authorship belongs in the
dedicated editing tool.
