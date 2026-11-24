---
name: sessions
description: List every provider-qualified session linked to an exact quest ID locally.
argument-hint: <quest-id>
disable-model-invocation: true
---

This command must be handled by the Fini native UserPromptExpansion hook, which
displays its deterministic shared-core result without invoking a model.
If you see this text, report that the hook is unavailable and stop. Do not run
tools, infer a quest or invent associations.
