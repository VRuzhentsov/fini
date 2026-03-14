# NewQuestForm

Thin wrapper that connects [[ChatInput]] to quest creation. Used in [[MainView]].

## Behaviour

On `submit` from [[ChatInput]], calls `createQuest({ title: text })` via [[quest.ts]]. Title only at capture time — other fields can be set later.

## Dependencies

| Dep | Role |
|---|---|
| [[ChatInput]] | Input UI, emits `submit` |
| [[quest.ts]] | `createQuest` |
