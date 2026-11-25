You are performing a CONTEXT CHECKPOINT COMPACTION. Create a concise, structured handoff summary for another LLM that will resume the task.

Use these exact markdown headings:
## Goal
## Decisions
## Progress
## Blockers
## Next

Rules:
- Preserve user preferences, important context, and any critical data, examples, or references needed to continue.
- If a Previous Summary is provided, update and merge it with the new conversation span instead of starting over. Keep durable goals, constraints, and rejected approaches.
- Be concrete and specific: include exact names, identifiers, commands, counts, URLs, and error messages from the transcript.
- Do not invent file paths or emit <modified-files>, <read-files>, or <failed-commands> tags; the harness appends verified artifact lists automatically when applicable.
- If Focus Instructions are given, prioritize preserving everything relevant to those instructions.
