# 0014 — Sync by state, with Loro

## Status

Accepted. The project owner chose Loro as the sync engine and the only store
(see "Decision"), answered all open questions, and approved the decisions.
It was written during the design discussion of 2026-11-24, and it records
what the project owner liked and rejected. It answers the research part of
issue #196. Nothing in it is implemented yet.

Paths are relative to the repository root. A line number points at the code
of the main branch on the day this record was written.

## Context

### The outbox grows with no limit

SpaceSync records each local change as one event and keeps the event:

| Table | Where | One row is | Removed |
|---|---|---|---|
| `sync_outbox` | `src-tauri/migrations/00000000000010_sync_and_focus/up.sql:35-49` | one change made here: the whole object as JSON | never |
| `sync_acks` | same file, `:52-57` | one change that one peer acknowledged; no foreign key to `paired_devices` | never, also not at unpair |
| `sync_seen` | same file, `:60-63` | one change from a peer, applied here | never |
| `tombstones` | same file, `:66-74` | one delete received here | after 30 days (`src-tauri/src/services/space-sync/apply.rs:278-290`) |

All reads and writes are in `OutboxRepository`
(`src-tauri/src/repositories/outbox.repository.rs`). Each write of a quest
records the full row (`record_quest`, `:317-368`). A rank rebalance records
all quests of the space (`:372-383`,
`src-tauri/src/repositories/quest.repository.rs:494-557`).

An acknowledged change stays in the outbox. So the growth is the same when
all peers are online, and a peer that is away adds the scan of its pending
events on each tick (`src-tauri/src/services/space-sync.service.rs:1102-1132`).

Measured on the JSON of a typical one-line quest: the payload is about
380 B, a `SyncEvent` frame is about 810 B (two Bluetooth fragments), and an
`Ack` frame is 64 B. With the indexes, the estimates on disk are:

| Row | For each | Estimate |
|---|---|---|
| `sync_outbox` | edit, on the device that made it | 0.7 KB |
| `sync_acks` | edit and peer | 0.2 KB |
| `sync_seen` | edit received | 0.1 KB |
| `sent_at` (memory) | peer and event, for the life of the process | 0.15 KB (`space-sync.service.rs:226,1037-1049`) |

At 30 edits a day with two peers, this is about 7.7 MB of outbox and 4.4 MB
of acks in a year, on each device. The first sync of a space sends all
history, oldest first (`space-sync.service.rs:881-890`,
`outbox.repository.rs:163-174`): about 11 000 events (9 MB) after one year,
where the rows of the space are about 300 KB.

### Defects

| Defect | Evidence |
|---|---|
| **The pending query fails in time.** It gives SQLite one bound value for each acknowledged event (`NOT IN (?, ?, …)`). The bundled SQLite accepts at most 32 766. After about 32 700 acknowledgements from one peer (about 3 years at 30 edits a day), that peer is expected to get no more changes. From the code; not reproduced. | `outbox.repository.rs:118-143` |
| **A delete has no LWW check.** A deletes q at t2; B edits q at t3 > t2. On A the edit brings q back; on B the delete removes it. The devices stay different. A received tombstone is never read. | `apply.rs:364-367`; `quest.repository.rs:1165-1176` |
| **The LWW compares with the wrong thing**: the receiver's own newest outbox event, not the stored row. An older change from a third device can win. | `apply.rs:369-374`, `outbox.repository.rs:145-160` |
| **No relay.** A received change does not go into the outbox, and the first sync sends only the device's own events. With pairs A-B and B-C, C never gets A's quests. | `apply.rs:356-506`, `outbox.repository.rs:163-174` |
| **A backup import does not sync.** It records no change. | `src-tauri/src/services/backup.service.rs` (no outbox call) |
| **The row clock is in whole seconds**, and a device with a slow clock loses with an edit that it made after it received the change. | `src-tauri/src/services/db.service.rs:115,129-133` |

### Constraints

- Sync is whole-row last-writer-wins (LWW) today
  (`SyncEventEnvelope::wins_over`, `src-tauri/src/services/channel.service.rs:716-733`).
  A checklist quest and a checklist day mark merge item by item against a base
  that each device keeps (`checklist_base`; #128; `apply.rs:339-449`,
  `quest.repository.rs:1226-1307`). This is a 3-way merge with a local base.
- Bluetooth carries iroh QUIC as GATT fragments of about 500 B
  (`docs/adr/0004-android-background-sync-daemon.md:127`), and 12 B on a weak
  link (`:149`). Devices are often offline.
- The data holds device ids: `checklist_activity.origin_device_id`
  (`src-tauri/src/schema.rs:63`) and the event origin
  (`channel.service.rs:706`).
- Each device has an Ed25519 key (the iroh key,
  `src-tauri/src/repositories/device.repository.rs:79-95`). Pairing pins the
  peer's key (`:140-176`), and the TLS handshake proves it on each link
  (ADR-0009 D8).

## Requirements stated by the project owner

| # | Requirement | What it means for the design |
|---|---|---|
| R1 | No storage growth from dead or silent peers | Nothing is stored for each change and peer. What is kept for a peer has a fixed size, or a time limit. |
| R2 | Find the difference between two clients quickly | The cost depends on the number of differences, not on the history or the size of the data. |
| R3 | Painless sync | No resend, acknowledgement or dedupe state to tune; it repairs itself after a bug, a restore or a reinstall. |
| R4 | Remove the outbox | No `sync_outbox`, `sync_acks`, `sync_seen`, and no `OutboxRepository`. |
| R5 | A must not learn that B synced with C | The data that A gets from B has no id, key or signature of C. |
| R6 | Relay A-B-C works | C gets A's changes through B with no direct pair. |
| R7 | Works offline; the sync cost stays small on slow links | Few bytes and round trips for each sync, and a small first sync, because Bluetooth is slow. Fini's own Bluetooth drivers carry any reliable byte stream (iroh QUIC too), so the transport is not a filter. |

## Conflict handling

Status: chosen by the project owner.

Principle: **Merge automatically and deterministically, and never lose data
silently; history replaces prompts.**

| Situation | Example | Handling |
|---|---|---|
| Different fields of one quest | The phone renames "Gym"; the PC moves its due date | Both are kept: a merge for each field |
| The same simple field (status, due date, priority, energy, rank) | Both devices change the priority | One change wins, the same on all devices: an edit made after the device saw the other wins; for concurrent edits, Loro's order decides (D13). The losing value goes into the history of the quest, so the person can restore it. No prompt |
| The same text field | Both devices edit the description | The description is a Loro text (D15): edits on different lines both stay, online or after days offline. A title is a short field: one change wins (Loro's order), and the other goes into the history |
| Delete against edit | The phone deletes the quest; the PC edits it | The newer action wins. A delete that wins moves the quest to "recently deleted", where the person can restore it |
| Complete against edit | The phone completes the quest; the PC moves its date | Both are kept: they are different fields |
| Both add checklist items | One adds "eggs", the other adds "milk" | Both lines stay: a checklist is lines of the description (D14), and the text merges (D15) |
| Both reorder | Both devices move the same quest in the list | The rank of each quest: the newer change wins |
| Tree moves that make a cycle | One device moves A under B; the other moves B under A | A deterministic rule removes the cycle, the same on all devices |
| The same repeating day completed twice | The phone and the PC both complete Monday of "Gym" | Idempotent: one completed mark |
| Counters | Both devices add 25 focus minutes | Sum: 50 minutes |

The history of a quest keeps the last 10 losing values of each field.

**Consequence.** Whole-row LWW, the rule of today, is ruled out. Each
candidate must merge field by field, and keep a bounded history of each
quest. The scores of the main candidates:

| Candidate | Merge for each field | History of a quest | Counter as a sum | Tree cycles |
|---|---|---|---|---|
| (a) Outbox, (b) and (c) as written (whole row) | ✗ | ✗ | ✗ | ✗ |
| (b′) Reconciliation, a version for each field | ✓ | A history table of its own (bounded as chosen) | A value for each device, summed | Rule of its own |
| (d) Automerge | ✓ | ✓ in its operation history, which has no bound | ✓ counter type | ✗ no tree type |
| (e) cr-sqlite | ✓ (each column) | ✗; a table of its own | ✗ | ✗ |
| (f) Git-style | ✓ (3-way) | ✓ the commits, bounded by the shallow history | A value for each device | Rule of its own |
| (h) iroh-docs | ✓ with one key for each field | The last entry of each author only | A key for each author, summed | ✗ |
| (i) Willow | ✓ with one path for each field | ✗ | A path for each author | ✗ |
| (j) CouchDB-style, (k) Couchbase Lite | ✗ by itself (whole document); the app merges the kept conflicts | The losing revisions, until compaction | ✗ | ✗ |
| (l) Ditto | ✓ | ✗ (not public) | ✓ counter type | ✗ |
| Loro | ✓ (each map key) | ✓ in its operation history, bounded by shallow snapshots | ✓ counter type | ✓ movable tree, deterministic |
| yrs | ✓ (each map key) | ✗ deleted content is removed (garbage collection) | ✗ | ✗ no tree type |

## Decision

**Guiding principle: keep the Loro integration thin.** Use what Loro does by
itself wherever possible, so that the migration has little custom code that
can break. A custom solution is not rejected, but it is deferred, to keep the
surface for bugs small during the migration. Each item below says which parts
are Loro's own and which custom parts are deferred and optional.

| Item | Loro's own (now) | Custom (deferred, optional) |
|---|---|---|
| D3 Sync | Version vectors, `export(updates)`, `import` | — |
| D4 Deletes | — (a field) | — |
| D5 History | Lost values read from Loro's history (checkout, diff) | An explicit list of the last 10 values |
| D6 Trimming | No trimming during the migration | Shallow snapshots, and the re-apply path for an old peer |
| D7 Relations | Decided in ADR-0015 | — |
| D8 Migration | Mergeable containers (`ensure_mergeable_map`; measured) | "The smaller id builds, the other applies its differences again" (no longer necessary) |
| D10 Storage | Snapshot and update files, `import` | In-memory indexes (necessary), integrity checks at read |
| D12 CLI | Import of each update file | The compaction rule for files; a short lock on the one CLI id |
| Concurrent writes | Loro's own order | The HLC wrapper: dropped |
| D14, D15 Checklists and descriptions | A plain child Loro text; progress counted from the text | The item merge of #128: removed; a merge of an edit made before the first comparison: deferred |
| D16 Repeating quests | The rule as one value; marks as mergeable maps keyed `<quest_id>/<date>` | Archiving old closed marks: deferred |
| D17 Reminders | Not in the space doc | A recompute after each import (necessary) |

### D1 — Loro is the sync engine

SpaceSync syncs space docs. The outbox goes (R4). The sections after
"Decision" record the options that were compared.

Considered, not chosen:

| Option | Reason |
|---|---|
| (b′) Fini's own field versions, history and hash-range reconciliation | No peer ids in the data, but Fini writes the reconciliation, the field versions, the tree rule and the counters itself |
| yrs | It needs Fini's own delete-set protocol to keep the payload small, and it has no history, no tree and no counter; its interface is 0.x |
| The other options | See "Sync approaches", "NoSQL options" and "Wider survey" |

### D2 — The accepted trade-off: peer ids in the data

Loro puts the peer id of the writer in each edit. So a device can see
that another writer exists, but not who it is. This relaxes "no device
identity in the data" (see "Trust and integrity"). The mitigation:

- On its first start, each installation makes two ids once and stores them:
  one for the app and one for the CLI (D12). Each is unique (a UUID), and a
  random 64-bit peer id is stored with it (Loro peer ids are 64-bit). The
  ids have no relation to the iroh device key or the Fini device id.
- The UI never shows a peer id.
- A writer loads its own earlier edits before it writes with its peer id
  again (Loro's rule). A store that is restored or copied to another device
  makes both ids again, so two writers never share one peer id.
- `checklist_activity.origin_device_id` leaves the synced data.

### D3 — Sync

```
for each mapped space (one space doc for each space, D11):
  device 1 ──▶ its version vector (about 10 B for each peer id)
  device 2 ──▶ its version vector
  each side ──▶ export(updates since the other's vector)  ──▶ import
```

- The frames go over the iroh session (`fini/sync/7`). One round trip finds
  and sends the difference (measured: 98 B for one edit, 1.6 KB for 100).
- A comparison runs after each local edit, after about 2 s with no more
  edits, and every 30 s while connected. There is no other push.
- Relay A-B-C works, because B's space doc has A's edits.

**How this works: collaborative editing that does not need a connection.**
Loro is the same kind of tool as the live co-editing of a shared document,
but it works asynchronously. Each device commits its edits locally, at once.
There are no transactions across devices and no locks. A sync at any time,
after seconds or after days, merges the two space docs, and the result is the
same on all devices. While two devices are connected, the comparison after
about 2 s makes the edits of one device appear on the other almost at once.

### D4 — Soft deletes

- A delete is a soft delete: it sets the field `deleted_at` of the quest; it does not remove the
  map key. A delete and an edit are then two fields, and both stay.
- "Recently deleted" lists the quests with `deleted_at`. A restore clears it.
- With Loro's order (D13), a delete and a concurrent edit both stay: the
  quest is deleted, with the edit in it, and a restore shows the edit.

### D5 — The conflict history comes from Loro's history

- Loro keeps each edit, so the values that lost are in its history. The
  app reads them with Loro's own functions (a checkout of an earlier version,
  or the diff between versions), and shows up to the last 10 values of a
  field. A restore writes the chosen value as a new edit.
- The history goes back only to the point of a shallow snapshot. With no
  shallow snapshots during the migration (D6), it is complete.
- Deferred, optional: an explicit list of the last 10 values of each field in
  the quest, appended at each write. It is necessary only if the history must
  survive a shallow snapshot.

### D6 — Shallow snapshots

- During the migration, the app writes no shallow snapshot. The cost is the
  full history in the snapshot (measured: 528 KB against 234 KB for 1 000
  quests after 10 000 edits). Then no peer is ever older than the point of a
  shallow snapshot.
- Deferred, optional: shallow snapshots. Then each device records the version
  vector that each active peer sent last (one vector for each peer), makes a
  shallow snapshot only at a point that all active peers have passed, and a
  peer that is silent for more than 90 days does not stop it. When such a peer
  comes back, its old edits are rejected (measured:
  `ImportUpdatesThatDependsOnOutdatedVersion`). It then computes its local
  changes since its last common version (Loro's diff), imports the new state
  into a fresh space doc, and writes those changes again as new edits. A
  shallow snapshot also removes the history that D5 reads.

### D7 — Quest relations

Quest relations are decided in ADR-0015; the engine choice does not preset
that model. The order of the quests stays the rank keys, a field of each
quest (D10).

### D8 — Migration

- Each device makes, for each mapped space, a space doc from its SQLite
  rows. Two devices with the same rows each make a space doc, and the two
  then merge at the first `fini/sync/7` comparison.
- Mergeable containers: Loro's Rust interface has `ensure_mergeable_map`,
  `_list`, `_movable_list`, `_text`, `_tree` and `_counter`. Two peers that
  call the same method with the same parent map and key get the same
  container, and their edits merge in it.
- Measured (1 000 quests from the same rows on two devices; then each device
  edits a different quest; then the merge):

  | How the space docs are made | Quests | Edits of both devices | Checklist items (5 on each device) | A counter (25 on each device) | Snapshot, one device → after the merge |
  |---|---|---|---|---|---|
  | `insert_container` (a new child each time) | 1 000 | Lost on one device: for each key, the child of one device wins, with the edits in it | 5 | 25 | 208 KB → 403 KB (both copies stay) |
  | `ensure_mergeable_map` for each quest, a mergeable MovableList for the checklist | 1 000 | Both kept | 10: duplicated, because both lists get 5 inserts | 50: doubled | 250 KB → 344 KB |
  | `ensure_mergeable_map` for each quest, the checklist as a mergeable map of items keyed by item id | 1 000 | Both kept | 5 | 50: doubled | 571 KB → 768 KB |

- So the migration uses `ensure_mergeable_map` for each quest (and each
  occurrence mark). The plain fields merge, because both devices write the
  same values. A counter is not made in the migration: the migrated
  `focus_enter_count` is a plain field, and a counter counts only the new
  enters.
- The checklist items of today become `- [ ]` and `- [x]` lines of the
  description, with no item ids (D14).
- The description becomes a Loro text (D15). Measured: two devices migrate
  the same description, then merge:

  | Way | The text after the merge | Edits before the first merge | Custom code |
  |---|---|---|---|
  | A mergeable text (`ensure_mergeable_text`) on both | Two times | — | None |
  | (a) A plain field; the first edit makes a mergeable text | Once, when the first edits come one after the other. Two times, when both devices make their first edit before they merge | Both kept | A little |
  | (b) The device with the smaller id makes the texts; the other waits | Once | Kept, when the other device does not edit before the texts arrive | A rule for who makes them, a wait state, and a merge for an edit made in the wait. With pairs A(1)-B(5)-C(2), A and C both make texts, and B gets two (reasoning, not measured) |
  | (d) Both devices migrate with the same fixed peer id | Once, when the data is the same | Both kept | Little. But when the data is different, two operations have the same id: measured, the two devices stay different |
  | (e) A plain child text (`insert_container`) on each device | Once: one child wins on both devices | The edits in the child that loses are not in the text. They stay in that child, which can still be read by its id | None |

- Chosen by the measurement and the thin principle: (e). The description is
  a plain child Loro text, made when the quest is made or migrated. After the
  first merge, there is one text, and edits on different lines merge in it.
  The risk is narrowed to one case: an edit of a description on a device
  between its migration and its first comparison with the peer whose text
  wins. That edit is not lost: it stays in the losing child, and the history
  (D5) shows it so that the person can restore it. Deferred, optional: merge
  it into the winning text by itself (a 3-way merge with the migrated text as
  the base).
- Deferred, optional (no longer necessary): the device with the smaller
  device id makes the space doc, and the other device writes its differences
  again.
- The SQLite file (with the old outbox tables) stays, not used, for one
  release; then the app removes it.
- Devices on `fini/sync/6` and `fini/sync/7` do not connect; the Device page
  shows a notice.

### D9 — Rust

Loro builds with the Rust of Fini's build images (measured). No toolchain
change is needed.

### D10 — Loro is the only store

There is no SQLite. The data of each space is a space doc on disk: a snapshot
and update files.

```
<data dir>/spaces/<space-id>/
├─ snapshot.bin              the snapshot (only the app writes it)
└─ updates/
   ├─ <peer-id>-<n>.bin      an update file of one writer
   └─ ...
<data dir>/local/            data of this device only (settings, pairs,
                             channels, mappings, scheduled notifications);
                             it never syncs
```

- The data is a tree of containers, as JSON: maps, texts and counters.
  Relations are by id, as in a NoSQL store: a quest map holds `space_id`, and
  the key of an occurrence mark holds `quest_id`. There are no foreign keys
  and no joins.
- The app code checks integrity. A dangling reference is handled when it is
  read: a local reminder whose quest is missing or has a soft delete is
  ignored, and an occurrence mark of such a quest is not shown.
- In-memory indexes are built at load (measured: 45 ms for 10 000 quests) and
  kept up to date from Loro's events. The queries run on them (measured: 2 ms
  or less).
- Quest relations are decided in ADR-0015 (D7).

A space doc:

```
space doc "<space-id>"
├─ meta: Map              name, created_at
├─ quests: Map            quest_id → Map (mergeable)
│   ├─ title, status, energy, priority, due, due_time, completed_at,
│   │  order_rank, created_at, updated_at, deleted_at, space_id
│   ├─ repeat_rule: one value   { preset, interval, unit, days_of_week,
│   │                             end, end_date, end_after }   D16
│   ├─ description: Text  (a plain child text; checklist lines are in it,
│   │                     D14, D15)
│   ├─ focus_enter_count  (the migrated count, a plain field)
│   └─ focus_enters: Counter   (new enters only, D8)
├─ occurrences: Map       "<quest_id>/<YYYY-MM-DD>" → Map (mergeable)   D16
│                         status, closed_at, title?, description? (Text),
│                         energy?, priority?, due_time?, created_at,
│                         updated_at
└─ (quest relations: decided in ADR-0015)

Not in the space doc: reminders, snoozes and notifications, which are local
to each device (D17).
```

### D11 — One document for each space

A space is the unit that a pair maps, so a document is one space. A move of a
quest to another space is a delete (`deleted_at`) in one document and a create
in the other.

### D12 — The CLI is a Loro peer, with no lock on the store

- Each writer (the app, each CLI write) writes its edits to an update file of
  its own, so no other process writes that file. A writer writes the file
  under a temporary name, and renames it to `<peer-id>-<n>.bin` when it is
  complete.
- A reader imports all complete update files. Loro's import does not depend
  on the order, and an import of the same edits again changes nothing.
- The app watches the `updates` folder and imports a new update file from the
  CLI in milliseconds.
- Only the app compacts: it writes a new snapshot (a temporary file, then a
  rename), then removes the complete update files whose edits the new
  snapshot holds. It never removes a temporary file, so it cannot remove a
  file that a writer still writes.
- The peer id of a CLI run. Measured: 2 000 CLI runs on a space of 1 000
  quests; each run loads the snapshot and the update files, makes one edit and
  writes its own file; the app compacts after each 100 runs:

  | Option | Version vector | Full snapshot | Shallow snapshot | Sync to a paired device that synced before the runs | CLI load (average) |
  |---|---|---|---|---|---|
  | A new random id for each run | 2 001 entries, 8.0 KB, also after a shallow snapshot | 499 KB | 340 KB | 195 KB, and the device sends 8.0 KB in each later comparison | 10 ms |
  | A pool of 4 CLI ids, each claimed with a lock file for that id | 5 entries, 17 B | 435 KB | 323 KB | 46 KB | 7.5 ms |
  | One CLI id with a lock file | 2 entries, 9 B | 431 KB | 321 KB | 42 KB | 6.7 ms |

  All three converge. A shallow snapshot does not remove the entries of old
  peers from the version vector.
- Chosen: one CLI id for each installation (D2), as the measured "one CLI id"
  row (about 9 B of version-vector data). Two writers must never write with
  the same peer id at the same time, so only a CLI write takes the lock:
  - A read (load the space doc, run a query) takes no lock: the update files
    are only appended and renamed into place, so a reader never sees a
    partial file.
  - A write takes an exclusive lock on the CLI id, loads the store again
    (which then holds the earlier edits of that peer id), writes its update
    file, and releases the lock.
  - A CLI command that must write while another one holds the lock waits until
    the lock is free, with no error, then writes. A CLI load is about 7.5 ms
    (measured), so a wait is short. A long timeout with a clear message is
    only a safety net.
  - The lock is a file lock of the operating system, not a file with a
    process id: when a process stops, the system releases its lock.
  - The lock is on the CLI id only: the app has its own id and never waits
    for the CLI.
- Rejected: a new id for each run (each run adds a version-vector entry for
  good, and each later comparison carries it), and a pool of CLI ids (not
  necessary with one id and a short lock).

### D13 — Concurrent writes of one field

Loro's own order: the higher Lamport time wins, then the higher peer id. An
edit made after a device saw another edit wins over it; for two concurrent
edits, the order is deterministic. All devices converge. The HLC wrapper is
dropped.

### D14 — A checklist is lines of the description

- There is no checklist entity. A checklist is Markdown task lines in the
  description: `- [ ] item` and `- [x] item`. The person edits them only in
  the editor; the list view has no tick.
- The progress (for example 3/5) is counted from the text.
- An item has no identity, so there are no reminders or links for one item.
- The migration turns the checklist items of today into `- [ ]` and `- [x]`
  lines of the description (D8).
- The item-by-item merge of #128 (`checklist_base`, `merge_3way`) and the
  checklist tables (`checklist_activity`) go away. The text merge of D15
  takes their place.

### D15 — The description is a Loro text

- The description of a quest is a Loro text: a plain child text
  (`insert_container`), made when the quest is made or migrated (D8). Edits
  and ticks on different lines from two devices both stay, when the devices
  are online and after days offline.
- A mergeable text is not used: two devices that each migrate the same
  description into one get it two times (measured, D8).
- This replaces the earlier choice "newer wins, with history" for long
  descriptions (see the discussion log).
- A title stays a short field of the quest map: one change wins, in Loro's
  own order (D13).

### D16 — Repeating quests: the repeat rule and the occurrence marks

The repeat rule and the occurrence marks are an important part of the data,
and both are in the space doc. A repeating quest is one quest with a rule
(ADR-0012 D1); its occurrences are computed from the rule (D2), and a mark is
stored only for a day that a person closed or changed (D3).

**The repeat rule today** (`RepeatRule`, `src-tauri/src/models/quest.rs:372-413`):
a JSON text in `quests.repeat_rule` with `preset` (`daily`, `weekdays`,
`weekends`, `weekly`, `monthly`, `yearly`, `custom`, or `none`), `interval`
and `unit` (`day`, `week`, `month`, `year`) for `custom`, `days_of_week`
(`Mo`…`Su`; with an interval, every N weeks from the week of the start), `end`
(`on_date` or `after_n`), `end_date` and `end_after`. The dates are computed
from the start, which is the quest's `due` (else its creation day), by
`compute_next_due` (`quest.rs:464-513`), for at most 10 000 steps.

**The marks today** (`quest_occurrences`,
`src-tauri/migrations/00000000000028_quest_occurrences/up.sql:11-24`;
`src-tauri/src/models/quest_occurrence.rs:20-37`): one row for each
(`quest_id`, `period_key`); no later migration changes it.

**How the code uses them** (`src-tauri/src/repositories/quest.repository.rs`):

| Act | What it writes | Where |
|---|---|---|
| Complete or Abandon | Closes one day (the given date, or the oldest open): `status`, `closed_at`; for a checklist, a copy of the day's text | `close_occurrence`, `:956-985` |
| Complete through today | Closes each open day up to today, each with its own mark (ADR-0012 D7) | `:1086-1116` |
| Open a day again | Clears `status` and `closed_at`; the mark goes if it keeps no override | `reopen_occurrence`, `:988-1019` |
| "This day" edit (`EditScope::Day`) | An override on the day's mark, kept only when it differs from the quest | `:845-876` |
| "All days" edit (`EditScope::All`) | The quest itself; a rule with other days (`same_days_as`) moves `due` to today, so the open days restart from today; the old marks stay (ADR-0012 D10) | `:877-899`, `quest.rs:386-391` |
| Stop the repeat | With no closed day: a one-time quest that keeps its open day's edits; with closed days: the rule ends at the last closed day, and the quest is completed | `stop_repeat`, `:1024-1082` |

Focus points at the quest and shows its oldest open occurrence; it writes no
mark (ADR-0012 D6). History shows the closed marks.

**All fields of today's tables:**

| Table | Field | In Loro | Derived | Local to the device | Removed |
|---|---|---|---|---|---|
| `quests` | `id` | the map key | | | |
| | `space_id`, `title`, `status`, `energy`, `priority`, `due`, `due_time`, `completed_at`, `order_rank`, `created_at`, `updated_at` | ✓ fields | | | |
| | `description` | ✓ a Text (D15) | | | |
| | `repeat_rule` | ✓ one value (below) | | | |
| | `focus_enter_count` | ✓ the migrated count; new enters in a Counter | | | |
| | `is_checklist` | | from the text (D14) | | |
| | `checklist_base` | | | | ✓ (#128) |
| | (new) `deleted_at` | ✓ (D4) | | | |
| `quest_occurrences` | `quest_id`, `period_key` | the key `"<quest_id>/<YYYY-MM-DD>"` | | | |
| | `status`, `closed_at`, `title`, `energy`, `priority`, `due_time`, `created_at`, `updated_at` | ✓ fields | | | |
| | `description` | ✓ a Text (the day copy) | | | |
| | `checklist_base` | | | | ✓ (#128) |
| `reminders` | `id`, `quest_id`, `type`, `mm_offset`, `due_at_utc`, `created_at`, `scheduled_notification_id` | | the reminder of a dated quest, from `due`, `due_time` and the rule | ✓ custom rows (D17) | |
| `notification_snoozes` | `reminder_id`, `fire_at_utc`, `created_at` | | | ✓ (D17) | |
| `focus_history` | `id`, `quest_id`, `space_id`, `trigger`, `created_at` | | | ✓ (each device has its own focus) | |
| `checklist_activity` | all | | | | ✓ (D14) |

Derived, never stored: the dates of the rule, the open occurrences, the
oldest open one, the day as the person sees it (`RepeatingQuest::on_day`,
`quest.rs:647-681`: each field with no override takes the quest's value, and
`due` is the date), the remaining count of `after_n`, and the History list.

**The rule in the space doc: one value.** `repeat_rule` is one value of the
quest map, with the same fields as today. Measured (50 repeating quests with
30 marks each; the same data on two devices; then A changes the interval and B
the weekdays at the same time):

| Layout | After the migration | After the concurrent edit |
|---|---|---|
| A mergeable map, `days_of_week` as a mergeable list | `days_of_week` = Mo, We, Mo, We (duplicated) | interval from A, days from B: a rule that no device wrote |
| A mergeable map, `days_of_week` as one value | correct | interval from A, days from B: a rule that no device wrote |
| One value (chosen) | correct | the whole rule of one device (Loro's order); the other rule is in the history |

The fields of a rule depend on each other (the preset decides which fields
count), so one value keeps a rule that a person wrote. The rule and `due` stay
two separate fields of the quest map, each in Loro's own order, with no custom
grouping. Accepted edge case (the thin principle): a rule change on one device
and a date change on another at the same time can pair the rule of one device
with the start date of the other. The project owner judged this not
important.

**The marks in the space doc.** A top-level map `occurrences`, keyed
`"<quest_id>/<YYYY-MM-DD>"`. Each mark is a mergeable map
(`ensure_mergeable_map`), so two devices that complete the same day write the
same entry (idempotent). A list is not used (lists duplicated in the spike).
Only the overrides that differ from the quest are stored, as today. To open
a day again, the device clears `status` and `closed_at` and keeps the key
(a removed key with a concurrent edit in it hides that edit).

**A rule change.** As today: a past day keeps its mark (and History shows it);
the open days come again from the new rule, from today. A mark of a date that
the new rule does not give stays as History.

**The migration.** Each device writes, from its tables, the rule as one value
and each mark with `ensure_mergeable_map` under its composite key. Measured,
50 repeating quests with 30 marks each on two devices: 1 500 marks after the
merge (no duplicates), and the same document on both.

**Measured** (10 daily quests, 365 completed days each, a Priority override
on every 10th day; the same marks on two devices; after the migration, A
changes the Priority of a day and B its due time; then the merge):

| How the marks are made | Marks after the merge | The two edits of the same day | Snapshot of one device |
|---|---|---|---|
| `insert_container` | 3 650 (no duplicates) | One lost: the child of one device wins | 225 KB (63 B a mark), shallow 140 KB |
| `ensure_mergeable_map` | 3 650 (no duplicates) | Both kept (Priority from A, due time from B) | 262 KB (73 B a mark), shallow 137 KB |

The same day completed on both devices at the same time gives one mark
(idempotent); its `closed_at` follows Loro's order. The first merge after the
migration holds the migration operations of both devices (387 KB after the
merge); a shallow snapshot (D6, deferred) would remove them.

**Conflicts.** The conflict table applies: the same field of one day follows
Loro's order (D13); different fields of one day are both kept.

**Growth.** One mark for each closed or changed day of each repeating quest:
about 73 B a mark with its history, 37 B in a shallow snapshot (measured). A
daily quest closed every day adds about 27 KB a year; 20 daily quests add
about 530 KB a year (estimate from the measured size). A rule is one value of
about 100 B, plus about 100 B of history for each change. This needs no trim
for years. Deferred, optional: archive the closed marks older than a limit
(for example two years) to a file outside the space doc.

### D17 — Reminders, snoozes and notifications are local to each device

- They do not sync. Each device derives its reminders from the synced quest,
  its `due`, `due_time`, repeat rule and occurrence marks (ADR-0012 D9), and
  keeps its snoozes and its own reminder rows locally (`<data dir>/local/`,
  D10).
- After each Loro import, the device computes its reminders again, and
  schedules or cancels its own notifications.
- This removes a defect of today: when A completes or deletes a quest, the
  alarm on B still fires. Evidence: after it applies peer changes, SpaceSync
  publishes only `Event::SpaceDataChanged`
  (`src-tauri/src/services/space-sync.service.rs:1290-1291`), and the reminder
  service does nothing for that event (`src-tauri/src/services/reminder.service.rs:300`,
  `Ok(_) => {}`).

### Follow-ups (answered)

| | Question | Answer |
|---|---|---|
| a | "Newer wins" | Loro's own order (D13) |
| b | Storage | Loro is the only store (D10) |
| c | Document size | One document for each space (D11) |
| d | The CLI | A Loro peer with no lock (D12) |

## Sync approaches

One row for each option. Rows (h) to (m) are the NoSQL options; see
"NoSQL options" below for their facts.

| Option | How | Storage growth | Finding differences | Deletes | Edits of different fields at the same time | Relay A-B-C | Privacy (device ids in the data) | Sync cost (bytes, round trips, first sync) | Fit with Fini's storage and queries | Complexity | Status |
|---|---|---|---|---|---|---|---|---|---|---|---|
| (a) Outbox + acks, pruning | Today's events; remove an event that all peers acknowledged; drop the queue of a peer away for N days, then a full sync | Bounded by the pruning; acks for each event and peer until then | The queue is the difference | Delete event in the queue | One wins (whole row) | No (defect) | Origin id in each event | 2 fragments + ack for each change | Already there | Low | Not chosen: keeps the outbox (R4) |
| (b) Hash-range state reconciliation, LWW for each row | One set of (version, id) items for each space; negentropy range fingerprints find the different items; send those rows | Rows + deletion markers (time limit) | O(d · log n); 1 round trip when equal | Markers with LWW; a time limit for silent peers | One wins; checklists merge as today | Yes: B's set has A's rows | None needed: the tie rule can use a content hash | Equal: 1 round trip, ~110 B each way; 1 change: 3–4 round trips, ~3 KB | Good: any store with a version for each row; SQL queries stay | Medium | Liked: "hashes of hashes", outbox removed |
| (b′) As (b), a version for each field | The same set; a row merges field by field | Rows + a version for each field + markers | As (b) | Markers | Both stay | Yes | No id needed | As (b); smaller rows | Good; more columns or a side table | Medium-high | Open |
| (c) Sequence numbers for each device | Each write gets the next local number; send "rows above N" for each peer | Rows + markers + one number for each peer | 1 round trip; exact only while the numbers are correct | Markers with numbers | One wins | Yes, if a received row gets a local number | Origin id to stop echo | Best: 1 round trip + rows | Good; triggers | Low-medium; silent drift after a restore | Not chosen alone: no self-repair (R3); a fast path later |
| (d) Automerge | A CRDT document of operations; sync messages for each peer | Full operation history | Bloom filter of heads | Delete operations in the history | Both stay; lists and text merge | Yes | Actor id in each operation | Compact; history makes the first sync large | Possible as the only store; queries in memory; history in the file | High | Discussed; not chosen |
| (e) cr-sqlite | SQLite extension; CRR tables; `crsql_changes WHERE db_version > N` | Rows + a clock for each column + markers | 1 round trip; numbers | Markers in its clock tables | Both stay (each column) | Yes | Site id of each writer | Small | SQL queries stay; but a loadable extension to build for each platform, and little activity since 2024 | High (packaging) | Not chosen |
| (f) Git-style | A Merkle tree of the state; commits; for each peer the last common commit; 3-way merge | Rows + one merge base for each peer + short history | Compare tree hashes from the top: O(d · log n) | No markers: in the base and missing on one side is a delete | Both stay (3-way) | Yes: B's commits hold the merge with C | Author in commits if stored; merge commits show C's hashes unless squashed | Like (b), plus commit data | Medium: base snapshots, commits, a merge step | High | Liked, under discussion |
| (g) Blockchain | A chain of signed blocks with consensus | Grows forever; cannot be pruned | Compare chains | Transactions | — | Yes | Keys of all signers | Large | Poor | Very high | Rejected |
| (h) iroh-docs | Signed entries in a namespace, keyed by (author, key); LWW for each (author, key) by wall-clock time; range-based set reconciliation | Entries for each author and key; deletion markers are never removed | O(d · log n) | Prefix delete: an empty entry for each (author, prefix); it stays | One wins (whole value) | Yes | Author key in each entry; one shared author key for all devices would hide C | Like (b) for the entries; each entry carries two signatures and the ids; the content is a second transfer (iroh-blobs) | Possible as the only store (redb); queries in memory; content as blobs | Medium-high | Under discussion |
| (i) Willow / Earthstar | Entries at (subspace, path, time); 3D range-based reconciliation; Meadowcap capabilities | Bounded: a newer entry at a prefix removes older entries below it, with no marker | O(d · log n) in 3D ranges | Destructive prefix edits | One wins (each path) | Yes | Subspace (author) ids in entries; private overlap detection hides what is not shared | Like (b), in 3D ranges; capabilities add bytes at the start of a session | Possible as the only store; but the Rust implementation is alpha and the sync design is paused | High | Under discussion |
| (j) CouchDB-style | Revision tree for each document; `_changes` since a checkpoint for each peer; `_revs_diff` | Revision tree for each document up to `_revs_limit`; tombstones stay; purge does not replicate | Checkpoint + `_revs_diff` | `_deleted` revision (tombstone) | Losing revisions kept as conflicts; app merges | Yes | Revision ids; no device id needed | JSON revisions with their history; several round trips for each batch; the first sync sends the revision trees | Possible as the only store (a young Rust implementation on redb); queries by views | High | Under discussion |
| (k) Couchbase Lite | CouchDB-style replication in a C library | As (j) | As (j) | As (j) | As (j) | Yes | As (j) | As (j) | Possible as the only store, with its own queries; but peer-to-peer only in the Enterprise Edition, and the Rust binding is unofficial | High; licence cost | Under discussion |
| (l) Ditto | Closed-source CRDT document store with a mesh over Bluetooth, LAN and Wi-Fi | CRDT metadata in its store | Its own protocol | CRDT deletes | Both stay (CRDT) | Yes (mesh) | Peer ids in its mesh | Not public | Possible as the only store, with its own queries; but closed | Low to integrate, high to depend on | Under discussion |
| (m) Generic embedded KV (redb, sled, SurrealDB embedded, RocksDB) | A store only | — | None; only a prolly tree on top gives a diff | — | — | — | — | — | A store only: it needs a sync on top, and the queries are written again | — | Under discussion |

Notes on the table:

- **(b) and "hashes of hashes".** Negentropy (the range-based set
  reconciliation of the Nostr relays, a Rust crate with an MIT license) sorts
  the items by time and compares one fingerprint for each range. When a range
  differs, it splits it into 16 sub-ranges, and it sends the ids of a small
  range. Its fingerprint is additive (the sum of the ids, then a hash), so a
  sub-range costs no new hash. It does the same job as a Merkle tree, and
  there is no tree to store. A stored Merkle tree (a real hash of hashes, as
  in (f)) is quicker to compare, but each write must update it.
- **Bluetooth estimates for (b)**, with 2 000 items in the space, rows of
  about 0.9 KB as a frame, 4 KB negentropy frames on Bluetooth, and 50–150 ms
  for each Bluetooth round trip (an assumption, to measure):

  | Rows that differ | Round trips | Bytes | Bluetooth fragments |
  |---|---|---|---|
  | 0 | 1 | ~220 B (all spaces in one frame) | 2 |
  | 1 | 3–4 | ~3 KB | 6–8 |
  | 100, recent | 3–4 | ~100–190 KB | 200–390 |
  | 10 000, one side empty | 1, then the rows | ~9 MB | ~18 000 |

  There is no live push (chosen): a comparison runs after each local edit,
  so one edit costs the 3–4 round trips of the table.
- **(f) git-style** is how the checklist merge works today, in small: a base,
  the local text and the incoming text (`merge_3way`). With a merge base for
  each peer, a delete needs no marker, and edits of different fields merge.
  The cost is one base for each peer (R1 then needs a time limit or a shared
  store), and the history between the oldest base and now ("shallow" history,
  as `git clone --depth`). Prior art: Dolt (a SQL database with prolly trees
  and cell-level 3-way merge), Fossil (SQLite storage; the sync exchanges the
  ids of the artifacts that each side has). A peer whose base is gone falls
  back to a 2-way merge, as (b) does after its time limit.
- **(b′)** is cr-sqlite's model (a clock for each column) without the
  extension. It merges edits of different fields with no merge base.

## NoSQL options

The facts were checked against the project pages and the source on the day
this record was written.

| | iroh-docs | Willow / Earthstar | CouchDB / PouchDB | Couchbase Lite | Ditto |
|---|---|---|---|---|---|
| **Status** | Released often by n0 (the iroh team); depends on the same major version of iroh as Fini | Specs active; the Rust crates are alpha; the Confidential Sync design is paused | Mature; the reference replication protocol | Mature product | Commercial product |
| **Data model** | Namespace (its key gives write access) → entries keyed by (author, key). An entry holds the BLAKE3 hash, the size and a timestamp of the content; the content is a blob | Namespace → subspace → path → entry (timestamp, payload digest) | A JSON document with a revision tree | As CouchDB, with an SQLite file | JSON documents with CRDT fields |
| **Signed** | Each entry by the namespace key and the author key | Meadowcap capabilities | No | No | Its own identity |
| **Conflict rule** | For each (author, key): the newer timestamp, then the content hash. Timestamps are wall-clock microseconds; entries more than 10 minutes in the future are rejected | For each (subspace, path): the newer timestamp, then the digest | A deterministic winner; losing revisions stay as conflicts | As CouchDB | CRDT merge |
| **Sync** | Range-based set reconciliation (the same family as negentropy) | 3D range-based set reconciliation over (subspace, path, time) | `_changes` since a checkpoint, `_revs_diff`, then the missing revisions | As CouchDB | Its own mesh protocol |
| **Deletes and GC** | `del(author, prefix)` writes an empty entry that clears that author's entries below the prefix. The empty entry stays; no GC of markers is documented | A newer entry at a prefix removes the older entries below it. No marker grows | `_deleted` revisions stay; compaction removes old bodies, not the tree; purge is local and does not replicate | As CouchDB | Not public |
| **Growth for each author** | One entry for each key that each author wrote: N authors writing a key keep N entries | One entry for each (subspace, path) | One tree for each document | As CouchDB | Not public |
| **Privacy (R5)** | The author id is in each entry. If each device has its own author, A sees C's author. One author key shared by all devices of the person hides C, but then entries do not show which device wrote | Subspace ids in entries; private interest overlap detection hides the namespaces and areas that only one peer has | Revision ids only | As CouchDB | Peer ids in its mesh |
| **Sync cost** | Range fingerprints, then entries of about 250 B plus the key (two signatures and the ids, estimate); the content is a second transfer through iroh-blobs | 3D range fingerprints; capabilities add bytes at the start of a session | JSON revisions with their history; several round trips for each batch | As CouchDB | Not public |
| **Licence** | MIT / Apache-2.0 | MIT / Apache-2.0 (the Rust crates) | Apache-2.0 | Source Apache-2.0; peer-to-peer sync and encryption only in the Enterprise Edition (commercial) | Proprietary; a free tier and paid tiers; it does not fit the AGPL licence of Fini (ADR-0009) without an exception |
| **Rust** | Native | Native (alpha) | No SQLite-backed Rust implementation; a young pure-Rust one uses redb | Unofficial community binding | Official SDK, closed source |

**Generic embedded KV stores** (redb, sled, SurrealDB embedded, RocksDB)
bring no sync. For diffs, only a content-defined Merkle tree on top matters:
a prolly tree (Dolt, noms). It gives the same tree for the same data in any
insert order, so two trees compare from the root, and equal subtrees are
skipped. A prolly tree is one possible form of the state tree of (f).

### Fit with Fini's storage and queries

Fini is not tied to SQLite or Diesel. Today the data is in SQLite and Diesel
reads it. The criterion is the fit with what Fini needs from its store:

- **Persistence**: writes that survive a crash.
- **Queries**: the sorted lists, overdue quests, the Focus pick, the
  occurrences of a day, the reminders that are due, and the quest relations
  (ADR-0015).
- **Schema evolution**: new and renamed fields, with old and new app versions
  on the paired devices at the same time.
- **Backup and import** (`backup.service.rs`).
- **CLI access**: the CLI is a separate process that writes the data today.

| | (i) The sync store is the only store (a NoSQL store or a CRDT document) | (ii) The sync store is the truth; SQLite is a view built from it | (iii) SQLite stays; the sync uses a KV model over rows (key = table/id) |
|---|---|---|---|
| **What changes** | All repositories and queries are written again on the new store | Each write goes to the sync store; a step copies it into SQLite tables | Only SpaceSync: each synced row is a (key, version, value) entry |
| **Queries** | In memory or with derived indexes. Measured: the list sort of 10 000 quests in about 2 ms (see "Loro and yrs up close") | SQL, on the view | SQL, as today |
| **Two stores** | No | Yes: two stores to keep the same, and a rebuild after a failure | No |
| **Schema evolution** | Fields in the document; no migration step, but renames need a rule | The view is built again | SQL migrations, as today |
| **Sync** | The store's own | The store's own | Option (b) or (b′) |
| **Risk** | Medium: all queries are written again, but they are small and quick | High: two stores | Low |

Option (iii) is the KV sync model with no second store, and it gives the
same design as (b). Option (i) is no longer penalized for a second store
when the sync store is the only store. iroh-docs and Willow are useful as
references for (iii): range-based reconciliation, signed entries, prefix
deletes.

## Wider survey

Other databases and sync libraries for local-first, offline or peer-to-peer
sync. Fini has no server, so an engine that needs one is rejected or only a
source of ideas. The transport is not a filter (R7). The R column scores
R1–R7: ✓ meets, ~ in part, ✗ does not.

### Viable for Fini

| Name | Kind | Language, embeddable | Server | Sync and diff | Deletes, growth | Conflicts | Privacy (R5) | Sync cost | Maturity | R1–R7 |
|---|---|---|---|---|---|---|---|---|---|---|
| Loro | CRDT library: map, list, text, movable tree | Rust; embeddable, also on Android | No | Updates since a version vector; any byte stream | CRDT history; shallow snapshots trim it, as a Git shallow clone does | CRDT merge for each field; list and tree moves | Random peer ids in operations; one id for each device shows C as an id | 1 round trip for the delta; the first sync is a snapshot | Active | 1~ 2✓ 3✓ 4✓ 5~ 6✓ 7✓ |
| yrs (y-crdt) | CRDT library, the Rust port of Yjs | Rust; embeddable | No | State vector, then the missing updates | Deleted content is removed; small metadata stays | CRDT merge for text, lists and maps | Random client ids in items | 1 round trip | Mature (Yjs) | 1~ 2✓ 3✓ 4✓ 5~ 6✓ 7✓ |
| p2panda | Peer-to-peer toolkit on iroh: signed append-only logs for each author, log sync | Rust; embeddable | No | Log sync for each author | Logs for each author | Left to the app (any CRDT) | Author keys in operations | Depends on the log length | APIs not stable yet | 1~ 2~ 3~ 4✓ 5✗ 6✓ 7~ |
| negentropy | Range-based set reconciliation (the engine of (b)) | Rust crate; embeddable | No | Range fingerprints | Left to the app | Left to the app | No ids | See the estimates of (b) | Used by Nostr relays | 1✓ 2✓ 3✓ 4✓ 5✓ 6✓ 7✓ (with (b)) |
| iroh-docs | See (h) | | | | | | | | | 1✗ 2✓ 3✓ 4✓ 5~ 6✓ 7~ |

### Ideas worth borrowing

| Name | The idea | For which option |
|---|---|---|
| TinyBase `MergeableStore` | An HLC and a hash for each value; hashes go up through rows and tables, so a diff walks down only where hashes differ; pluggable synchronizers | (b′): a version for each field with a three-level hash tree |
| Rateless IBLT (Yang, Gilad, Alizadeh) | The sender streams coded symbols; the receiver decodes the difference after about 1.35 × d symbols, with no round trip for each level | (b): one message for a small difference on Bluetooth, negentropy as the fallback |
| Merkle Search Tree (Auvolat, Taïani) | A stored Merkle tree whose shape depends only on the keys; equal subtrees have equal hashes | (b) or (f): a stored "hash of hashes" |
| Prolly trees (Dolt, noms) | A content-defined Merkle tree; Dolt merges cell by cell, 3-way | (f) |
| Evolu | Range-based reconciliation with end-to-end encryption; the relay sees only owner ids, timestamps and encrypted data | R5: one owner id for each person, no device ids in the synced data |
| Actual Budget | An HLC and a Merkle trie over the change timestamps | The HLC; not its message log, which grows |
| Corrosion (Fly.io) | cr-sqlite rows, SWIM membership, and a periodic sync with a few peers | A periodic check as a safety net |
| Syncthing (BEP) | An index exchange with a version vector for each file | (f); but the vectors hold device ids (R5) |
| Fossil | The sync exchanges the ids of the artifacts that each side has ("igot", "gimme") | (f) |
| SQLite session extension | Changesets: compact row diffs with a conflict handler | A row diff encoding; it needs a base |
| sqlite-sync (SQLite AI) | A CRDT kind for each table: causal-length set, delete-wins, add-wins, grow-only | The delete rule for each table |
| diamond-types | A fast text CRDT | Descriptions and checklist text |
| Hypercore | Signed Merkle-tree logs | Proofs of who wrote a row |

### Rejected

| Name | Reason |
|---|---|
| ElectricSQL, PowerSync, Zero, Replicache, InstantDB, Triplit, Convex | Server-centric: a server is the source of truth (Replicache is also archived) |
| WatermelonDB, LiveStore, Jazz | Sync through a server (pull and push, or a sync backend) |
| Turso / libSQL embedded replicas and offline sync, sqlite-sync | Sync to a cloud primary or a server backend (sqlite-sync stays an idea) |
| Litestream, LiteFS, rqlite, mvsqlite, Marmot | Server replication: one primary, Raft, FoundationDB or NATS; not for offline peers |
| Yjs (JavaScript) | Use yrs in Rust |
| OrbitDB, Gun, Hypercore / Autobase / Hyperbee, Earthstar, DXOS, RxDB, TinyBase, Evolu | JavaScript or TypeScript only; operation logs grow (OrbitDB, Hypercore), or a relay or master is needed (Evolu, RxDB); TinyBase and Evolu stay ideas |
| any-sync (Anytype) | Go; needs sync nodes |
| Dolt, TerminusDB, Irmin, Fossil | Servers or apps in Go, Prolog or OCaml, not a library for Rust; Dolt and Fossil stay ideas |
| SurrealDB, redb, sled, RocksDB | Stores with no peer-to-peer sync (see (m)) |

## Loro and yrs up close

The numbers come from a spike outside the repository: a release build in a
Linux x86_64 container, one run each, so the times are approximate. The
spike also compiled for `aarch64-linux-android`; it did not run on a phone.
Values marked "estimate" were not measured.

### Data model

| Fini data | Loro | yrs |
|---|---|---|
| Containers | Map, List, MovableList, Text, Tree, Counter | Text, Array, Map, XML |
| A quest | A map for each quest, in a root map keyed by quest id; one key for each field | The same, with Y.Map |
| Checklist items | A MovableList of maps: adds merge as a union, moves are native | An Array of maps: a move is a delete and an insert |
| Order | The rank key as a field (as today), or a MovableList of quest ids | The rank key as a field |
| Parent tree (a comparison only; relations are decided in ADR-0015) | Tree: a movable tree with a deterministic cycle rule, siblings ordered by fractional index | A `parent_id` field; the app needs its own cycle rule |
| Occurrence marks | A map for each quest, keyed by `period_key` | The same |
| Reminders | A map in the quest | The same |
| Spaces | One document for each space (the sync unit is a mapped space); a move to another space is a delete in one document and a create in the other | The same |
| Counters (focus minutes) | Counter: concurrent adds are summed | A value for each device, summed by the app |

### Measured: 1000 quests, then 10 000 edits, then 300 deletes

Each quest has 11 fields and 5 checklist items.

| Measure | Loro | yrs |
|---|---|---|
| Document after the build | 409 KB | 544 KB |
| After the edits and deletes, all history kept | 528 KB (full snapshot) | 692 KB (garbage collection off) |
| After the edits and deletes, trimmed | 234 KB (shallow snapshot) | 414 KB (garbage collection on, the default) |
| Update for 1 edit | 98 B | 15.3 KB; 8.7 KB in the v2 encoding |
| Update for 100 edits | 1.6 KB | 16.2 KB |
| The delete set, sent with each update | — | 15.3 KB (it grows with each overwrite and delete) |
| The sync request (version vector or state vector) | 5 B | 5 B |
| 10 000 local edits | 10 ms | 9 ms |
| Merge of 100 offline edits from a peer | 2.4 KB, 9 ms | 2.5 KB, 0.2 ms |
| Catch-up of a peer that was silent through all the edits and deletes | 146 KB, 16 ms | 81 KB, 11 ms |

### Measured: load and queries (fresh process)

| Measure | Loro, 1 000 quests | Loro, 10 000 quests | yrs, 1 000 quests | yrs, 10 000 quests |
|---|---|---|---|---|
| File (shallow snapshot or state) | 318 KB | 3.3 MB | 544 KB | 5.6 MB |
| Load and first full read | 5 ms | 62 ms | 12 ms | 150 ms |
| Memory of the document and the index | +8 MB | +28 MB | +12 MB | +122 MB |
| Build the in-memory index from the document | 3 ms | 45 ms | 3 ms | 44 ms |
| List sort, on the index | 0.1 ms | 1.7 ms | 0.1 ms | 1.6 ms |
| Focus pick, on the index | 0.01 ms | 0.1 ms | 0.01 ms | 0.08 ms |
| Reminders due, on the index | < 0.01 ms | 0.05 ms | < 0.01 ms | 0.04 ms |
| Start from a shallow snapshot and 1 000 logged updates (97 KB) | 8 ms one by one; 2 ms as a batch | — | — | — |
| Size added to the Android binary (aarch64, stripped) | +3.8 MB | | +0.6 MB | |

On a mid-range phone, the estimate is the same memory and two to four times
the Linux times: for 10 000 quests, about 120–250 ms to start with Loro and
300–600 ms with yrs (estimate).

### Sync, relay and cost on slow links

Both use one round trip: a device sends its version vector (Loro) or state
vector (yrs), and gets the operations that it does not have. Both work A-B-C
with no server, because B's document holds A's operations.

| Changes that differ | Loro | yrs |
|---|---|---|
| 0 | the vector (5 B) and a near-empty update (estimate) | the vector, and an update with the whole delete set (15.3 KB here) |
| 1 | 98 B | 15.3 KB |
| 100 | 1.6 KB | 16.2 KB |
| 10 000 edits and 300 deletes | 146 KB | 81 KB |
| First sync (1 000 quests) | 234–528 KB snapshot | 414 KB |

### History and old peers

- **Loro** keeps every operation, so it can show and restore old values
  (time travel). A shallow snapshot removes the history before a point.
  Measured: a peer whose version is older than that point gets
  `ImportUpdatesThatDependsOnOutdatedVersion` for its own updates. It can
  still get the new state (updates or a snapshot), but its offline edits made
  before the point are not merged; the app must apply them again as new edits.
  So trim only when all active peers are past the point (the rule of the
  deletion markers), and give a peer that is silent for more than 90 days the
  new state, then its edits again.
- **yrs** removes deleted content (garbage collection), but each deleted item
  keeps a small record, and the delete set grows (15.3 KB after 10 000
  overwrites) and goes with each update. It has no trim point, so an old peer
  can always sync. It keeps no old values, so the history of a quest needs a
  structure of its own.

### Privacy (R5)

Both put a random 64-bit writer id in each operation (Loro peer id, yrs client
id). Measured: after relay A-B-C, A's version vector holds C's id. The id is
random and not the device key, so A cannot tie it to C; but A sees that a
third writer exists.

- A new random id for each session hides even that a writer is the same
  device. Measured cost after 500 sessions with one edit each: a Loro snapshot
  of 28.4 KB against 2.7 KB with one id, and a version vector of 5.3 KB against
  23 B (10.6 KB after a shallow snapshot).
- An id must never write at the same time as another writer with the same id.
  A device that uses an id again must first load its own earlier operations
  with that id. If not, two different operations get the same id, and the
  replicas become different.

### Conflicts, measured or from the docs

| Situation | Loro | yrs |
|---|---|---|
| Different fields | Both kept | Both kept |
| The same field at the same time | The higher Lamport time, then the higher peer id: a causal order, not the newer wall-clock time. The losing value stays in the history | A logical order with the client id as the tie rule, not time. The losing value is removed |
| Delete against edit | A deleted map key removes the quest with the concurrent edit in it (the edit stays in the history) | Measured: an edit on a deleted quest is lost; the quest stays deleted on both |
| Concurrent moves that make a cycle | Measured: device 1 put A under B, device 2 put B under A; both devices end with A under B and B at the root | No tree type |
| Counters | Summed (Counter) | No counter type |
| Two devices create a child container at the same key | One child wins and the content of the other is lost from the state (Loro docs); a "mergeable" child container avoids it | Similar |

For the chosen policy ("Conflict handling"), a delete must be a field (for
example `deleted_at`), not a removed key. Then a delete and an edit are two
fields, and "recently deleted" can restore the quest. "The newer by HLC wins"
for the same field needs an HLC stored with the value, because both libraries
order concurrent writes by logical time, not by wall-clock time.

### Integration

| | The CRDT document is the only store | CRDT document + an SQLite view | CRDT for text fields only |
|---|---|---|---|
| **Persistence** | A snapshot file and an append-only log of updates. Each update is appended with its length and a checksum, then synced to disk; a torn last record is dropped. Compaction writes a new snapshot to a temporary file, renames it, then empties the log. Measured start: 2–8 ms for 1 000 logged updates | As the first, plus the SQLite file | SQLite, with the CRDT bytes of each text field in a column |
| **Queries** | An in-memory index built at load (45 ms for 10 000 quests) and kept up to date from the document events. Measured queries: 2 ms or less | SQL, on the view | SQL |
| **Schema evolution** | A new field is a new key; an old app version keeps the keys that it does not know. A rename writes the new key and reads the old key too. A migration that two devices run at the same time must not create child containers at the same key | The view is built again | SQL migrations |
| **Backup and import** | Export a snapshot; import merges it or replaces the store | The same | As today |
| **Desktop app and CLI** | One owner process (the background service) holds the document, and the CLI sends its commands to it; or a file lock, where each process has its own writer id, appends to the log, and imports the records of the other | The CLI must write through the document owner, not into SQLite | As today |
| **Memory and start, 10 000 quests** | Loro +28 MB, 62 ms on Linux; on a phone 120–250 ms (estimate) | As the first, plus SQLite | Small |
| **Risk** | All storage and queries are written again | Two stores to keep in step, and a rebuild when they differ | Sync of the other fields still needs (b′) |

### Maturity

| | Loro | yrs |
|---|---|---|
| Licence | MIT | MIT |
| Releases | 1.x, several each month | 0.x, a release that can break the interface every one to three months |
| Main authors | One main author and one second | One main author |
| Builds with the Rust of Fini's build images | Yes | The newest release needs a newer Rust. This is a small cost: the Rust version in `containers/android/Containerfile` and `containers/linux/Containerfile`, `docs/container.md`, and the `msrv` of ble-gatt's `clippy.toml`. CI uses stable Rust already |
| Size added to the Android binary | +3.8 MB | +0.6 MB |

### Smaller yrs payloads (measured)

The same dataset: 1 000 quests, 10 000 edits, 300 deletes. The delete set is
15.3 KB and has one writer.

| Technique | No change | 1 edit | 100 edits | CPU | Correct (the peer converges) |
|---|---|---|---|---|---|
| v1 update (the default) | 15.3 KB | 15.3 KB | 16.7 KB | encode 0.1–0.3 ms | Yes |
| v2 update | 8.7 KB | 8.7 KB | 9.7 KB | about the same | Yes |
| v1 + zstd (level 1) | 11.7 KB | 11.7 KB | 12.7 KB | +0.03 ms | Yes |
| v2 + zstd (level 1) | 2.4 KB | 2.5 KB | 3.3 KB | +0.03 ms | Yes |
| v2 + lz4 | 4.5 KB | 4.5 KB | 5.3 KB | less than zstd | Yes |
| Blocks only, plus an 8-byte hash of the delete set | 10 B | 29 B | 1.4 KB | as v1 | Yes for map overwrites: the receiver deletes the old value itself when it adds the new one, so the delete sets stay equal. No for a removed map key or a removed list item: the hashes then differ |
| Blocks only, and on a hash difference the delete-set ranges of the buckets that differ | — | — | — | small | Yes. For 10 removed quests: 6.0–13.6 KB with buckets of 256–4 096 clocks, plus 0.1–1.4 KB of bucket hashes |
| Blocks only, and on a hash difference only the missing ranges (set reconciliation over the ranges) | — | — | — | small | Yes. For 10 removed quests: 331 B |
| One Y.Doc for each quest | — | 82 B for that doc | — | small | Yes. But 539 KB in all (against 414 KB), and finding the changed docs needs a digest: 19–24 KB for 1 000 docs as a list, or about 1–3 KB with range reconciliation over (doc id, hash) (estimate) |
| Fields as registers (a Y.Array of values, not a map overwrite) | — | 23 B with all values kept; 1.2–2.6 KB when the old values are removed (1 or 10 kept) | — | small | Yes. Keeping all values grows the state with each edit; removing old values brings the delete set back |
| Compaction (a fresh doc built from the current state) | delete set 1 B | — | — | 4 ms | Only if all devices change to the new doc at the same time. Measured: a peer with the old doc that merges it keeps both copies (396 KB), and its offline edit is lost |

The yrs interface gives what these need: the delete set as an `IdSet` (with
encode, decode, `diff` and `merge`), and a v1 update is the blocks followed by
the delete set. A "blocks only" update is that update with an empty delete
set, and a "delete set only" update has no blocks.

**The best combination**: (1) a delete is a field (`deleted_at`, which the
policy needs for "recently deleted" anyway), and a checklist item is removed
the same way, so the data has almost no explicit removes; (2) sync sends the
blocks only, with a hash of the delete set; (3) when the hashes differ, send
the missing delete-set ranges, found by range reconciliation; (4) compress
with zstd only above a size limit. Measured result: 10 B with no change, 29 B
for one edit, 1.4 KB for 100 edits. This is close to Loro (98 B, 1.6 KB), but
it is Fini's own protocol on top of yrs.

### Verdict (before the decision; Loro was chosen, D1)

- **Loro for everything** is the strongest CRDT candidate. It merges field by
  field, keeps a history that a shallow snapshot bounds, has a counter and a
  movable tree with a cycle rule, and its updates are small on slow
  links (98 B for one edit). The costs: storage and queries written again (the
  measured query times are small), 3.8 MB in the APK, random writer ids that
  show that a third writer exists (R5 in part), logical time instead of the
  HLC for the same field, and offline edits from before a trim point that the
  app must apply again.
- **yrs as it is** is not recommended: each update carries the whole delete
  set (15.3 KB here). With the combination above it costs 10–29 B for a small
  sync, but Fini then owns a sync protocol on top of yrs. It still has no
  history, no tree and no counter, and its interface is 0.x. Long descriptions
  get no text merge (chosen), so yrs has no special use left for text.

### Loro or (b′): the trade-off (Loro chosen, D1)

(b′) is Fini's own design: a version (HLC) for each field, the history of
each field, and hash-range reconciliation of the items.

yrs is in the table for comparison, with the payload combination above.

| | Loro | (b′) | yrs, with the payload combination |
|---|---|---|---|
| **Writer ids in the data** | A random writer id in each operation. After relay A-B-C, A can see that a third writer exists (not C's key). This conflicts in part with "no device identity in the data" | None: the item is (version, hash); the tie rule uses the content hash | A random client id in each item, as Loro |
| **Sync payload (measured)** | 98 B for 1 edit, 1.6 KB for 100 | 3–4 round trips and about 3 KB for 1 edit (estimate) | 10 B with no change, 29 B for 1 edit, 1.4 KB for 100 |
| **Storage** | The document is the store (or a store with an SQLite view); measured +28 MB and a 3.3 MB file for 10 000 quests | SQLite stays; a version for each field and a history table (10 values for each field) | The document is the store; measured +122 MB and a 5.6 MB file for 10 000 quests |
| **History and old peers** | The history is in the operation log; a shallow snapshot trims it, and a peer older than the trim point must apply its offline edits again | The history table is separate from the sync, so nothing blocks an old peer; deletion markers have the 90-day rule | No history (a table of its own); no trim point, but the delete set only grows; a compaction needs all devices at the same time |
| **Concurrent writes to one field** | Logical time; an HLC must be stored with the value for "the newer by HLC wins" | The HLC directly | Logical order with the client id; the same need for an HLC |
| **Tree, counters, list moves** | Built in (movable tree, Counter, MovableList) | Written by Fini (a cycle rule, a value for each device, rank keys) | Written by Fini |
| **Build effort** | Storage, queries, persistence and the CLI access written again; the sync comes with the library; +3.8 MB in the APK | The reconciliation, the field versions, the history and the tree rule written by Fini; storage and queries stay | As Loro, plus Fini's own delete-set protocol; +0.6 MB in the APK; a newer Rust (a small cost) |

## Trust and integrity

Status: chosen. Authenticated iroh links only: no signature for each row and
no device identity in the data. Signatures can come later if a relay that is
not trusted appears. With Loro, the data holds random writer ids, which
relaxes "no device identity" (D2).

| | Authenticated iroh links only (no signature for each row) | Signed rows, original signer | Signed rows, signed again at each hop |
|---|---|---|---|
| **How** | The TLS handshake proves the paired peer on each link (today). Rows carry no signature. | The device that wrote a version signs it. A relay sends the signature unchanged. | Each device signs the rows that it sends with its own key. |
| **Proves who wrote the row** | No. It proves only which paired device sent it. | Yes, through any number of hops. | No. It proves only the last hop, which the link already proves. |
| **Hides C from A (R5)** | Yes, if the rows hold no device id. | No. A sees C's key, and needs it to verify (C must be introduced to A). | Yes. |
| **Storage and CPU** | None. | About 100 B for each row (signature and key). Ed25519 verify about 50–150 µs on a phone; a first sync of 10 000 rows about 0.5–1.5 s (estimates). | The same storage. A sign and a verify at each hop. |
| **Relay and digests** | No effect. | The signature is the same on all devices, so it can be in the item id or out of it. | The signature changes at each hop, so it must stay out of the item id and the digests. If not, each hop looks like a difference. |
| **Revocation at unpair** | Stop the link (today). | The revoked key's rows must be kept or signed again by another device. | Stop the link. Stored signatures of the revoked device stay as history only. |

An unpair is local to this device: the link to that device stops, and the
data that the unpaired device wrote stays.

## Version clock

| | Wall clock (today) | Hybrid logical clock (HLC) |
|---|---|---|
| **What it is** | The device time of the write | The larger of the device time and the newest version that the device has seen, with a counter for the same millisecond (Kulkarni et al.; CockroachDB, Actual Budget) |
| **An edit after a received change** | Can lose if the clock is slow | Always wins |
| **Two writes in the same millisecond** | Same version | Different versions |
| **A device with a clock far ahead** | Its edits win until the time catches up | It pulls the other clocks forward; the order stays correct |
| **Reads as a time** | Yes | Yes, for devices whose clocks agree |
| **Status** | — | Chosen before Loro; dropped with Loro (D13) |

With Loro, the order of concurrent writes is Loro's Lamport time (D13), with
no wall clock and no HLC wrapper. The earlier choices for the options that
use a clock (an HLC; rejecting a version more than 24 hours ahead, with a
warning on the Device page) apply only to those options.

## Deletion markers and silent peers

These apply to (b), (b′), (c) and (e). Option (f) needs no markers while a
merge base exists. With Loro (chosen), quests use soft deletes (D4), and the
90-day rule applies to the trim point of the history (D6).

Chosen so far: if deletion markers are used, a peer that is silent for more
than 90 days stops blocking the removal of the markers.

Candidate (not decided):

- Remove a marker when every active peer of the space has reconciled after the
  marker was stored. This is exact.
- When a peer comes back after the 90 days, a row that it offers, that is not
  here, that is not newer than its last sync with this device, and that was
  written by it or by this device, was deleted here. This device sends a marker
  in place of inserting the row.
- Other rows follow the LWW. In two edge cases, both after more than 90 days
  away, a deleted row can come back. Data is never deleted.
- With no device identity in the data (chosen), the rule cannot ask who wrote
  a row. It then applies only to rows that are not newer than the last sync of
  the two devices, and a row from a third device with a slow clock can be
  treated as deleted. This needs a fix before it is chosen.

## Sync behavior

| Topic | Status | Rule |
|---|---|---|
| When a comparison runs | Chosen | After each local edit (with about 2 s of wait for more edits), and every 30 s while connected. No live push: only comparisons |
| Devices on different sync versions | Chosen | A notice on the Device page |
| The old outbox tables | Chosen | Kept, not used, for one release; then dropped |
| Unpair | Chosen | Local to this device; the data that the unpaired device wrote stays |
| What syncs | Chosen | The space names and imported backups sync. The settings stay on each device, and the focus history is local to each device (each device has its own focus, because the spaces differ for each device) |

## Discussion log

| Topic | Status | Reason |
|---|---|---|
| Merkle tree, "hashes of hashes" | Liked | Finds the difference from the state alone, in a few round trips (R2, R3) |
| Remove the outbox | Liked | It is the cause of the growth (R1, R4) |
| Git-style merge base (f) | Liked, under discussion | Deletes with no markers, and field merges, as the checklist merge does already; the cost is a base for each peer |
| Blockchain (g) | Rejected | No parties that do not trust each other; an append-only log cannot be pruned |
| Automerge (d) | Discussed, not chosen | For: merges field by field, merges lists and text. Against: the history grows, a second data store beside SQLite, actor (device) ids in each operation (R5) |
| 90-day limit for silent peers | Chosen so far | Only if deletion markers are used (R1) |
| Versions more than 24 h in the future | Chosen so far | Reject the row, and show a warning on the Device page |
| HLC | Dropped with Loro (D13) | Loro's own order also lets an edit made after a received change win, and all devices converge |
| Thin Loro integration | Chosen: the guiding principle | Little custom code during the migration; custom parts are deferred |
| Concurrent writes (follow-up a) | Chosen: Loro's own order | No HLC wrapper |
| Storage (follow-up b) | Chosen: Loro is the only store | Snapshot and update files for each space; relations by id; indexes in memory |
| Document size (follow-up c) | Chosen: one document for each space | A space is the unit of a mapping |
| The CLI (follow-up d) | Chosen: a Loro peer with no lock | Its own update file for each writer; only the app compacts |
| The CLI peer id | Chosen: one id for each installation, made at the first start; CLI runs take turns with a short lock on that id (replaces the pool of 4) | Measured: one fixed id adds about 9 B of version-vector data; a new id for each run gave 2 001 entries (8.0 KB) after 2 000 runs, which a shallow snapshot does not remove |
| The app peer id | Chosen: one id for each installation, made at the first start | The app never waits for the CLI; a restored or copied store makes both ids again |
| Migration of two devices with the same data | Chosen by measurement: `ensure_mergeable_map` for each quest | Plain containers lose the edits of one device; mergeable lists duplicate items and counters double, so the checklist stays text and the counter counts only new focus enters |
| The explicit 10-value history list | Deferred, optional | Loro's own history gives the values while the document is not trimmed |
| Trimming (shallow snapshots) | Deferred, optional | No trimming during the migration |
| Trust model | Chosen: authenticated iroh links only | No signature for each row and no device identity in the data; signatures can come later for a relay that is not trusted |
| Relay signed again at each hop | Rejected | No signatures for each row |
| Merge unit: row, field, or 3-way with a base | Chosen: a merge for each field | Whole-row LWW is ruled out (see "Conflict handling") |
| Conflict handling policy | Chosen | Merge automatically and deterministically; never lose data silently; history replaces prompts |
| History bound of a quest | Chosen: the last 10 losing values of each field | A fixed bound |
| Long descriptions | Replaced: the description is a Loro text (D15) | Earlier choice "newer wins, with history"; now edits on different lines from two devices both stay |
| Checklists | Chosen: lines of the description (D14) | No checklist entity; progress from the text; no per-item reminders or links; #128's item merge goes away |
| Titles | Chosen: one change wins (Loro's order) | A short field; the other value goes into the history |
| Description migration | Chosen by measurement: a plain child text on each device (D8) | A mergeable text doubles the text; a first-edit text doubles it when both devices edit first; a fixed migration peer id makes the devices different when the data differs; one child wins, and an edit before the first merge stays readable in the losing child |
| Occurrences | Chosen: in the Loro document (D16) | Mergeable maps keyed `<quest_id>/<date>`; measured: no duplicates, both edits of one day kept, 73 B a mark |
| Reminders, snoozes, notifications | Chosen: local to each device (D17) | Derived from the synced quest, rule and marks after each import; this fixes the alarm that still fires on B after A completes or deletes the quest |
| The repeat rule | Chosen by measurement: one value of the quest map (D16) | A mergeable map mixes two concurrent rule edits into a rule that no device wrote, and a mergeable list duplicated the weekdays |
| Sync scope | Chosen | Space names and imported backups sync; settings stay on each device; focus history is local to each device |
| The CLI write lock | Chosen | Only CLI writes take the exclusive lock on the CLI id; reads take none; a write waits for the lock with no error; a long timeout as a safety net; an operating-system file lock, released when a process stops |
| Focus history | Chosen: local to each device | Each device has its own focus, because the spaces differ for each device |
| Quest relations | Decided in ADR-0015 | The engine choice does not preset the model (ADR-0015 can choose parent and child, a graph of blocks, or both) |
| Unpair | Chosen: local to this device | The data that the unpaired device wrote stays |
| Live push | Rejected | Comparison only: after each local edit (about 2 s of wait), and every 30 s while connected |
| Version mismatch | Chosen | A notice on the Device page |
| Old outbox tables | Chosen | Kept, not used, for one release; then dropped |
| Loro or (b′) | Chosen: Loro (D1) | The project owner accepts the random writer ids (D2); (b′) and yrs are considered, not chosen |
| iroh-docs (h) | Under discussion | Same stack as Fini, and the same reconciliation; but author ids in entries (R5), markers that stay, and a second store |
| Willow / Earthstar (i) | Under discussion | Prefix deletes with no marker growth and private overlap detection; the Rust code is alpha and its sync design is paused |
| CouchDB-style (j) | Under discussion | Mature model; revision trees and tombstones grow; no SQLite-backed Rust implementation |
| Couchbase Lite (k) | Under discussion | Peer-to-peer only in the commercial edition; unofficial Rust binding |
| Ditto (l) | Under discussion | A finished mesh product; closed source, paid tiers, conflicts with AGPL |
| Loro | Chosen: the sync engine (D1) | Measured: 98 B for one edit, +28 MB and 62 ms for 10 000 quests; history, counter and movable tree; history bounded by shallow snapshots |
| yrs | Considered, not chosen | Measured: each update carries the whole delete set (15.3 KB); with blocks only, a delete-set hash and soft deletes: 10–29 B, but Fini then owns that protocol; no history, tree or counter; 0.x interface |
| A newer Rust | A small cost | The build images, `docs/container.md` and ble-gatt's `msrv`; CI uses stable already |
| Fini tied to SQLite | Rejected | The criterion is the fit with Fini's storage and queries; a sync store can be the only store |
| p2panda | Under discussion | Rust, on iroh; but its APIs are not stable, and author keys are in the data (R5) |
| Rateless IBLT | Under discussion | One message for a small difference, which suits a slow link |
| A stored Merkle tree (Merkle Search Tree, prolly tree) | Under discussion | The "hashes of hashes" in a stored form |
| A version and a hash for each value (TinyBase) | Under discussion | A model for (b′) |
| Server-centric engines | Rejected | Fini has no server |
| A concurrent rule change and `due` change | Accepted edge case: Loro's defaults (D16) | `repeat_rule` and `due` stay separate fields in Loro's own order; the project owner judged the pairing not important |
| Generic KV (m) and the fit analysis | Under discussion | No sync by itself; a NoSQL source of truth (i) or a second store (ii) costs all queries; (iii) is (b) |

## Open questions

None at this time. The items that are deferred and optional are listed in
the table of the guiding principle ("Decision").
