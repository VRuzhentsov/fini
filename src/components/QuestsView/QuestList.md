# QuestList

Shared quest list used in [[QuestsView]] and [[HistoryView]]. Rendering adapts per `quest.status`.

## Props

| Prop | Type | Description |
|---|---|---|
| `quests` | `Quest[]` | Quests to display — any mix of statuses |

## Row states

Each row has two states: **collapsed** and **expanded**. Tapping a row toggles it. Only one row is expanded at a time.

## Active quests (state: in progress)

### Collapsed
Checkbox + title.

### Expanded
**Header** — checkbox · editable title · golden pin (!) · collapse chevron

**Body** — editable description textarea

**Footer**
- Left: clock button + due/time/repeat hint — opens [[ReminderMenu]]
- Right: Attachment _(future)_ · Label _(future)_ · Flag (priority) · Energy · ⋮ (Abandon / Delete)

## Resolved quests (status: completed or abandoned)

### Collapsed
Checked checkbox (green = completed, amber = abandoned) · timestamp badge · strikethrough title.

### Expanded
**Header** — checked checkbox · strikethrough title · timestamp badge · collapse chevron

**Body** — description (read-only)

**Footer** — ⋮ (Make active / Delete)

Clicking the checkbox restores the quest to `active`.

## Dependencies

| Dep | Role |
|---|---|
| [[quest.ts]] | `updateQuest`, `deleteQuest` |
| [[ReminderMenu]] | Due date / time / repeat picker (active quests only) |
