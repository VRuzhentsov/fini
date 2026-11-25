# 0015 — A quest tree: each quest has a parent id

## Status

Proposed. Written during the design discussion of 2026-11-24. The project
owner chose an unlimited tree. This record compares the variants and gives a
recommendation for each one. Nothing in it is implemented yet.

## Context

All quests are in one flat list today. A person cannot put a quest below
another quest. The person wants a quest hierarchy: a quest can have child
quests, a child can have its own children, and the depth has no limit.

What exists now, and what the tree must work with:

- **Schema.** `quests` has no link to another quest
  (`src-tauri/src/schema.rs:35-53`, table made in
  `src-tauri/migrations/00000000000028_quest_occurrences/up.sql:124-142`).
  SQLite's `DROP COLUMN` rejects a column with a foreign key, so a column
  with `REFERENCES` makes a later change expensive (same file, lines 5-7).
  `quest_occurrences` and `reminders` delete with their quest through
  `ON DELETE CASCADE` (`.../00000000000028_quest_occurrences/up.sql:12`).
- **Rank keys.** `order_rank` is a rank key: text that compares byte by byte,
  with a key between any two keys, so a move writes one row
  (`src-tauri/src/models/rank.rs:1-21`; migration
  `00000000000030_quest_rank_key/up.sql:27-31`). `Rank::between` gives the
  key between two neighbours (`rank.rs:45-61`), `Rank::spread` gives the keys
  of a rebalance (`rank.rs:65-78`), and `Rank::MAX_KEY_LEN` starts a
  rebalance (`rank.rs:39`).
- **Sort.** `Quest::rank_order` compares the key, then Priority, then
  `created_at`, then the id (`src-tauri/src/models/quest.rs:258-267`).
  `Quest::list_order` puts overdue quests first
  (`quest.rs:249-253`); the store has the same order (`activeOrder`,
  `src/stores/quest.ts:159-166`).
- **Move and rebalance.** `QuestRepository::move_quest` finds the neighbours
  in one global list of all quests (`rank_list`,
  `src-tauri/src/repositories/quest.repository.rs:466-475`), writes one key
  (`quest.repository.rs:497-517`), and on a rebalance gives all quests new
  keys (`quest.repository.rs:545-572`). A new quest goes to the bottom of all
  quests (`bottom_rank`, `quest.repository.rs:449-460`). The places are
  `QuestPlacement::{Before, After, Top, Bottom}` (`quest.rs:192-205`).
- **Reading.** The repository loads all quests into memory for each list
  (`all`, `quest.repository.rs:591-607`) and for each move (`rank_list`).
  The Focus fallback is the first quest in the backlog order
  (`focus`, `quest.repository.rs:623-642`; `src/views/FocusView.md:40`).
- **Repeating quests.** A repeating quest is one row; its occurrences are
  computed from its rule and its occurrence marks (ADR-0012 D1, D2;
  `RepeatingQuest`, `quest.rs:531-689`). The per-day steps of a repeating
  quest are its checklist (ADR-0012 D5).
- **Sync.** A change is one `SyncEventEnvelope` with the whole row as its
  payload (`src-tauri/src/services/channel.service.rs:703-714`;
  `OutboxRepository::record_quest`,
  `src-tauri/src/repositories/outbox.repository.rs:318-368`). The newer
  `updated_at` wins the whole row (`wins_over`, `channel.service.rs:716-732`;
  the gate in `src-tauri/src/services/space-sync/apply.rs:356-374`). A quest
  that changes its space is a delete in the old space and an upsert in the
  new one (`outbox.repository.rs:327-366`). A delete writes a tombstone
  (`apply.rs:234-276`). SpaceSync carries only mapped spaces. The protocol
  version is in `SYNC_ALPN`, now `fini/sync/6`
  (`channel.service.rs:490-496`).
- **Repository trait.** Each repository is made on one connection and holds
  only that connection (`src-tauri/src/repositories/mod.rs:28-31`). The tree
  code goes in `QuestRepository` and in the `Quest` model, not in a new
  repository.
- **CLI.** `fini quest list`, `create` and `update` (with `--before`,
  `--after`, `--top`, `--bottom`) are in
  `src-tauri/src/services/cli.service.rs:139-271`. `list` sorts and filters
  a flat list (`cli.service.rs:842-859`). Output is text for a person, and
  JSON with `--json` (`specs/cli/README.md:7-17`;
  `cli.service.rs:1677-1716`).
- **UI.** `QuestList.vue` moves a row by drag-and-drop: a mouse after a move
  of 8 px, a finger after a long press of 350 ms
  (`src/components/QuestsView/QuestList.vue:22-30`, `81-91`). The drop finds
  the row under the pointer by its Y position only (`QuestList.vue:113-123`),
  and keeps the neighbour in the same overdue part (`QuestList.vue:132-145`).
  The store shows the move before the backend answers (`withMove`,
  `quest.ts:176-187`; `moveQuest`, `quest.ts:248-257`).
- **Sync redesign.** Issue #196 researches a sync that finds what to send
  from the database (a watermark per device and space, or a state
  comparison) in place of the outbox. It also asks if the merge stays whole
  row or becomes per field.

## Options compared

### 1. Storage model

| Model | Read the tree | Write cost of a move | Sync with whole-row LWW |
|---|---|---|---|
| **Adjacency list**: one column `quests.parent_id` | Children: an index on `parent_id`. Ancestors and descendants: `WITH RECURSIVE` | One row: `parent_id`, `order_rank`, `updated_at` | One event for one row. Two moves of different rows can make a cycle; a read rule repairs it (section 2) |
| **Closure table**: `quest_paths (ancestor_id, descendant_id, depth)`, one row for each pair | Ancestors and descendants: one indexed query, no recursion | Delete and insert (size of the subtree) x (depth) rows | Bad. The pairs are derived data. Pairs from two moves on two devices mix into a set that is not a tree. Each device must make the table again from a parent id, so the table is only a cache of the adjacency list |
| **Materialized path**: `quests.path = '/<id>/<id>/'` | Subtree: a prefix match on an index; ancestors: read the text | Each row of the subtree gets a new path | Bad. One move is one event for each descendant. A row that loses LWW keeps an old path that does not agree with its parent's path. A quest id is a UUID of 36 characters, so paths become long |
| **Nested sets**: `lft`, `rgt` integers | Subtree: one range query | New numbers for approximately half of all rows | Worst. One move writes many rows, and two moves on two devices break the intervals |

SQLite has `WITH RECURSIVE`, and Fini builds a bundled SQLite
(`libsqlite3-sys` with `bundled`, `src-tauri/Cargo.toml:60`). Diesel has no
DSL for a common table expression, so a recursive query is a
`diesel::sql_query`, as the code does already
(`src-tauri/src/commands/quest.commands.rs:604`). Fini keeps the data of one
person: hundreds or a few thousand quests. The repository loads all quests
for each list already, so the tree is built in memory in one pass. The
read speed of the other models gives no gain at this size.

**Recommendation: the adjacency list.** It is the only model in which a move
writes one row. That agrees with whole-row LWW and with the rank keys, which
also write one row for a move.

`parent_id` has no SQL foreign key:

- Sync can write a child before its parent (the events of a tick are
  limited, and spaces sync one by one). A foreign key rejects that write.
- `ON DELETE CASCADE` deletes children on a peer with no sync event and no
  tombstone of its own.
- A column with a foreign key cannot be dropped later (see Context).

### 2. Integrity

#### Cycles

**Local check.** Before a write that sets `parent_id = P` on quest `Q`, the
repository rejects `P = Q` and each `P` that has `Q` as an ancestor:

```sql
WITH RECURSIVE ancestors(id) AS (
    SELECT parent_id FROM quests WHERE id = :new_parent
    UNION
    SELECT q.parent_id FROM quests q JOIN ancestors a ON q.id = a.id
)
SELECT EXISTS (SELECT 1 FROM ancestors WHERE id = :moved);
```

`UNION` (not `UNION ALL`) ends the recursion when the stored data has a
cycle already.

**Cycles from sync.** Device 1 moves A below B. Device 2 moves B below A at
the same time. These are changes to two different rows, so LWW keeps both,
and after sync both devices have the cycle A -> B -> A. Options:

| Option | How | Result |
|---|---|---|
| Reject the incoming move | The receiver keeps its own `parent_id` | Device 1 keeps A below B, device 2 keeps B below A. The devices do not converge. Rejected |
| Repair on apply | The device that sees the cycle writes `parent_id = NULL` on one quest and records a sync event | The stored data is clean. Both devices can write the same repair, with new `updated_at` values, so one more sync round follows |
| **Repair on read** | The tree builder computes a *shown parent* for each quest. In a cycle, one quest is shown at the top level. Nothing is written | Each device has the same rows after LWW, so each device shows the same tree. No extra events |

**Recommendation: repair on read, with a deterministic rule.** In each
cycle, the quest with the oldest `(updated_at, id)` is the LWW loser: it is
shown at the top level, with its own subtree below it. The newer move stays
as the person made it. When a person next edits or moves that quest, the
write stores its shown parent, and the stored cycle is gone.

`updated_at` changes for each edit, not only for a move. An edit of the
title after the move makes that quest "newer". The rule stays deterministic,
but it does not always keep the newest move. A `moved_at` column fixes this
at the cost of one more column (open question 3).

#### Missing or deleted parent after sync

The parent of a quest can be not there: it did not arrive yet, it was
deleted on another device, or it is in a space that this device does not
sync.

| Option | Result |
|---|---|
| Clear `parent_id` on apply | A child that arrives before its parent loses its parent for ever |
| Keep the child hidden | The quest cannot be seen, and Focus cannot reach it |
| **Show at the top level, keep `parent_id`** | The quest is visible. When the parent arrives, the tree is correct again with no write |

**Recommendation: show at the top level, keep `parent_id`.** The same
*shown parent* rule applies to a list that shows only some quests: in the
active list, an active quest whose parent is closed is shown below its
nearest active ancestor, or at the top level.

#### Depth

**Recommendation: no limit in the data, the CLI or the sync.** The UI limits
only the indent (section 5). The tree builder and each recursive query stop
at a quest that they saw already, so a stored cycle cannot make an endless
loop.

### 3. Semantics

#### Complete a parent that has open children

| Option | Result |
|---|---|
| Block | Simple. The person must complete each child first. A child that a peer adds after the parent closed still gives an open child below a closed parent |
| **Cascade, with a confirmation** | One action closes a project. Each open descendant gets its own completed row and its own sync event, in one write transaction |
| Allow, children stay open | The children stay in the active list below no visible parent. Hard to understand |

**Recommendation: cascade, with a confirmation** ("Complete 'X' and its 3
open child quests?"). The CLI has no dialog, so `fini quest complete`
rejects a quest with open children unless the person adds
`--with-children`. A child that a peer adds below a closed parent follows
the *shown parent* rule.

#### Complete the last open child

| Option | Result |
|---|---|
| Complete the parent automatically | Writes a row that the person did not ask for. Two devices can race. A parent is often a project that gets more children later |
| **Do not complete the parent** | The parent becomes a leaf again, so Focus can show it (see Focus below). The UI shows a toast with the action "Complete 'X' too" |

**Recommendation: do not complete the parent automatically.**

#### Delete and abandon

| Option | Result |
|---|---|
| **Cascade to the subtree** | Each descendant gets its own delete event and tombstone, so each peer deletes each row. Its marks and reminders go with it, as today |
| Move the children up to the parent of the deleted quest | Keeps the work, but the person sees the children move to a different place |

**Recommendation: cascade, with a confirmation that gives the count.** The
person who wants to keep the children moves them first. Abandon cascades in
the same way as Complete. The CLI needs `--with-children` for `delete` and
`abandon` too.

#### Repeating quests in the tree

| Option | Result |
|---|---|
| Allow everywhere | A one-time child of a repeating parent completes once, for all days. The per-day steps of a repeating quest are its checklist already (ADR-0012 D5) |
| **Restrict: a repeating quest can be a child, not a parent** | A project can hold a daily habit ("Learn Spanish" > "Practice 15 minutes"). A repeating quest gets no children, and a quest with children cannot get a repeat rule |
| Forbid | Simple, but the person cannot put a routine in its project |

**Recommendation: restrict.** A repeating child is always a leaf. When its
parent closes by cascade, the repeat of the child stops at its last closed
occurrence (ADR-0012 D10).

#### Spaces

| Option | Result |
|---|---|
| A child can be in a different space | SpaceSync carries only mapped spaces. A peer that maps only the child's space gets the child with no parent. The tree is different on each device |
| **The parent and its children share one space** | One subtree syncs as one unit |

**Recommendation: one space for each subtree.** A new child takes the space
of its parent. A move below a parent in another space moves the subtree to
that space. A change of the space of a parent moves its whole subtree: each
row is a delete in the old space and an upsert in the new one, as today
(`outbox.repository.rs:339-366`). A change of the space of a child makes it a
top-level quest in the new space. A parent in another space after sync
follows the *missing parent* rule.

#### Focus: which quest is next

| Option | Result |
|---|---|
| Flat, as today | A parent can be Focus while its children are still open: the person sees a project, not a step |
| Parent first | The same problem |
| **Leaf first** | Focus shows a step that the person can do now |

**Recommendation: leaf first.** The Focus fallback walks the active tree in
order (pre-order, siblings in their list order). Among the active leaves (a
quest with no active children), it chooses the first overdue leaf, else the
first leaf. This keeps the rule "overdue first" of `src/views/FocusView.md:24`.
Set Focus and reminders do not change: a person can set Focus on a parent,
and a parent's reminder brings the parent to Focus.

#### Energy and Priority

| Option | Result |
|---|---|
| **Each quest keeps its own** | No hidden change. Sync stays one row for each change |
| Copy from the parent when the child is made | Quick to enter, still one value for each row |
| Computed from the children (sum or largest) | A parent shows the size of its project, but its own value cannot be set, and a change of a child changes what the parent shows |

**Recommendation: each quest keeps its own Energy and Priority. A new child
gets the values of its parent as defaults**, and the person or the CLI flags
can change them. A computed total can be added later as a display only.

### 4. Ordering

| Option | Result |
|---|---|
| One global rank | The tree order is the global key order inside each sibling group. A key between two siblings can be next to keys of other groups, so it works. But a rebalance gives new keys to all quests in all spaces: one sync event for each quest |
| **Rank keys for each sibling group** | `order_rank` compares only among quests with the same `parent_id` (and the same space). A rebalance writes only the siblings |

**Recommendation: rank keys for each sibling group.** The rank module does
not change. The changes:

- `rank_list` becomes the list of the siblings: the quests with the same
  `parent_id`. `Quest::rank_order` and `Quest::list_order` sort one sibling
  group. Overdue first stays in each sibling group.
- `move_quest` takes a parent and a place. `Before(id)` and `After(id)` take
  the parent of that neighbour. `Top` and `Bottom` are the top level. Two new
  places, `FirstIn(id)` and `LastIn(id)`, put the quest first or last below
  quest `id` (a drop onto a quest). On the wire: `{"first_in": id}`,
  `{"last_in": id}`.
- A move does the cycle check, then writes `parent_id`, `order_rank` and
  `updated_at` on one row. A rebalance calls `Rank::spread` for the siblings
  only.
- A new quest goes to the bottom of its sibling group (`bottom_rank` for
  one `parent_id`).
- The keys of today stay as they are. All quests are top-level quests after
  the migration, so the top level keeps its order with no new keys.

### 5. UI

- **Nested list.** `QuestList.vue` shows each row with an indent for its
  depth. The indent grows to a cap (5 levels is the proposal); deeper rows
  keep the indent of the cap and show their depth in the breadcrumbs. A
  parent row has a control to collapse and expand it, and a count of its
  open and all children ("2/5"). The collapsed state is a setting of the
  device, not a field of the quest, so it does not sync (open question 5).
- **Drag to a new parent.** The row under the pointer has three zones on
  the Y axis: the top quarter puts the moved quest before that row, the
  bottom quarter after it (a sibling), the middle half onto it (`LastIn`, a
  child). The row of the target shows which zone is active. A move of a
  parent carries its subtree. A drop onto the moved quest or one of its
  descendants is not shown as a target, and the backend rejects it too. A
  collapsed row opens after the pointer stays on it for a short time. The
  long press of 350 ms on a finger stays as it is. A horizontal drag to
  change the depth was considered and rejected: it is hard to do with a
  finger on a narrow screen.
- **Context menu.** "Move to..." opens a parent picker with search, and
  "Move to top level" makes the quest a top-level quest. These give a second
  way when a drop with a finger is not exact.
- **Editor.** `QuestEditor.vue` shows breadcrumbs above the title: the space,
  then each ancestor. A tap on an ancestor opens it. A "Parent" field opens
  the same parent picker, with "None (top level)".
- **History.** A closed child shows its breadcrumbs as text, so the person
  knows which project it was in.

### 6. CLI

- `fini quest create --title T --parent <id>`: the parent must exist and
  must not repeat. The child takes the space of its parent; a different
  `--space-id` is an error. Without a placement, the child goes to the
  bottom of its siblings.
- `fini quest update --id X --parent <id>` or `--root` (one of the two, in
  one clap group): moves the quest below `<id>`, or to the top level. With
  `--before`, `--after`, `--top` or `--bottom`, the place is in the new
  sibling group. `--before <id>` or `--after <id>` alone takes the parent of
  that neighbour.
- `fini quest complete`, `abandon` and `delete` reject a quest with open
  children, and give their count, unless the person adds `--with-children`.
- `fini quest list` gives the quests in tree order. The text output indents
  each depth by two spaces. `--json` keeps the flat `quests` array and adds
  `parent_id` and `depth` to each quest, so scripts of today still work.
  `--json --tree` gives a nested array (`children` in each quest).
  `--parent <id>` lists one subtree. A quest whose parent the filter
  removes follows the *shown parent* rule.

### 7. Sync and protocol

- **Column.** A new migration adds `quests.parent_id TEXT` (`NULL` is the
  top level), with no foreign key, and an index on
  `(parent_id, order_rank)`. `Quest` gets `parent_id: Option<String>` with
  `#[serde(default)]`, so a backup or an outbox event from before this
  change reads as a top-level quest. `upsert_external_quest`
  (`quest.repository.rs:279-323`) and `insert_export_quest`
  (`quest.repository.rs:47`) write it.
- **Envelope and LWW.** No change. A move is one upsert of one row, and
  `wins_over` compares it as it compares each other change.
- **Version.** `SYNC_ALPN` goes to `fini/sync/7` (`channel.service.rs:496`).
  A device on version 6 does not know `parent_id`: it would keep each
  incoming child as a top-level quest, and its next edit of that quest
  would send the row back with no parent. The new
  version makes the TLS handshake fail between the two versions, so this
  cannot occur. An old and a new device do not sync until both have the new
  version, as in ADR-0012.
- **Repair is on read.** The cycle rule and the missing parent rule are a
  function of the stored rows, in the `Quest` model. They do not depend on
  how the rows came. The outbox of today and a state-based sync give the
  same tree.
- **Interaction with the sync redesign (#196).**
  - A state-based sync sends whole rows, so `parent_id` goes with the row
    and needs no extra work. A cascade (complete, abandon, delete, a move to
    another space) is N changed rows, the same as N edits.
  - If #196 chooses a merge for each field, `parent_id` and `order_rank`
    must merge as one group: a rank key has a meaning only in its sibling
    group. A `parent_id` from one device with an `order_rank` from another
    device puts the quest in a random place.
  - The two changes each need a new protocol version. The change that is
    merged first takes version 7; the other takes the next number. Neither
    change needs to wait for the other.

## Decision

The recommended combination:

- **D1 — Adjacency list.** `quests.parent_id`, nullable, no foreign key,
  indexed with `order_rank`. A move writes one row.
- **D2 — Integrity.** A local recursive check rejects a cycle. A cycle from
  sync and a missing parent are repaired on read: the *shown parent* of the
  quest becomes the top level (in a cycle, the quest with the oldest
  `(updated_at, id)`). The stored `parent_id` stays until the next local
  write. The depth has no limit; the UI limits only the indent.
- **D3 — Close and delete cascade, with a confirmation.** Complete, Abandon
  and Delete of a parent close or delete its open subtree; the CLI needs
  `--with-children`. The last child does not complete its parent.
- **D4 — Repeating quests are leaves.** A repeating quest can be a child,
  not a parent. A cascade stops its repeat (ADR-0012 D10).
- **D5 — One space for each subtree.**
- **D6 — Focus is leaf first:** the first overdue active leaf in tree order,
  else the first active leaf.
- **D7 — Energy and Priority stay on each quest.** A new child copies them
  from its parent as defaults.
- **D8 — Rank keys for each sibling group.** Moves and rebalances find and
  write only the siblings. New places `FirstIn` and `LastIn`.
- **D9 — UI and CLI** as in sections 5 and 6.
- **D10 — Protocol version 7.** No change to the envelope or to LWW.

## Consequences

- A move, also to a new parent, is one row and one sync event. A rebalance
  writes only one sibling group.
- Each reader of the list (lists, Focus, History, CLI, backup) reads a tree
  from the same tree builder, with the same *shown parent* rule.
- A cascade writes one row for each quest in the subtree, in one
  transaction.
- After a sync conflict, a stored cycle or a missing parent can stay in the
  data until a person edits the quest. Each recursive query must stop at a
  quest that it saw already.
- The cycle rule uses `updated_at`, which changes for each edit. It is
  deterministic, but it does not always keep the newest move.
- A device on version 6 and a device on version 7 do not sync.
- New words need glossary entries (open question 6).

## Open questions

1. Repeating quests: restrict them to leaves (recommended), or forbid them
   in the tree?
2. Complete and Abandon of a parent: cascade with a confirmation
   (recommended), or block until each child is closed?
3. The cycle rule: use `updated_at` (recommended, no new column), or add a
   `moved_at` column that changes only for a move?
4. The indent cap of the nested list: 5 levels?
5. The collapsed state of a parent: a setting of each device (recommended),
   or a synced field?
6. Glossary: approve the terms *parent quest*, *child quest*, *top level*,
   *sibling group* and *shown parent*?
7. `fini --json quest list`: flat with `parent_id` and `depth` (recommended),
   and nested only with `--tree`?
8. Which change goes first, this tree or the sync redesign (#196)? Both take
   the next protocol version.
