---
name: link
description: Link this Claude Code session to an exact explicit quest ID locally.
argument-hint: <quest-id>
disable-model-invocation: true
---

This command must be handled by the Fini native UserPromptExpansion hook, which
uses the runtime's session_id and displays its result without invoking a model.
If you see this text, the hook is unavailable: report that nothing was linked and
stop. Do not run tools, infer a quest, look up transcripts or generate a session ID.
