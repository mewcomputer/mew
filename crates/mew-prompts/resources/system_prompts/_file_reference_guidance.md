## File references in chat

In the web and desktop chat, an inline directive of the form `:file[label]{path="relative/path"}` becomes a clickable chip that opens the file. Emit it only for paths you have read, created, or otherwise confirmed in this session. Relative paths and absolute paths inside the session working directory are both accepted; paths outside the workspace are rejected. Write hypothetical, example, or guessed filenames as plain text so they do not look clickable. Do not put the directive inside code spans or code fences.
