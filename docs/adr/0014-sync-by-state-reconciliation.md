# 0014 — Sync by state, with Loro

## Status

Accepted. The project owner chose Loro as the sync engine and the only store,
answered all open questions, and approved the decisions. Written during the
design discussion of 2026-11-24. It answers the research part of issue #196.
Nothing in it is implemented yet.

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

An acknowledged change stays in the outbox, so the growth is the same when
all peers are online. A peer that is away adds a scan of its pending events
on each tick (`src-tauri/src/services/space-sync.service.rs:1102-1132`).

Measured on the JSON of a typical one-line quest: the payload is about 380 B,
a `SyncEvent` frame is about 810 B (two Bluetooth fragments), and an `Ack`
frame is 64 B. With the indexes, the estimates on disk are:

| Row | For each | Estimate |
|---|---|---|
| `sync_outbox` | edit, on the device that made it | 0.7 KB |
| `sync_acks` | edit and peer | 0.2 KB |
| `sync_seen` | edit received | 0.1 KB |
| `sent_at` (memory) | peer and event, for the life of the process | 0.15 KB (`space-sync.service.rs:226,1037-1049`) |

At 30 edits a day with two peers, this is about 7.7 MB of outbox and 4.4 MB
of acks in a year, on each device (estimate). The first sync of a space sends
all history, oldest first (`space-sync.service.rs:881-890`,
`outbox.repository.rs:163-174`): about 11 000 events (9 MB) after one year,
where the rows of the space are about 300 KB (estimate).

### Defects

| Defect | Evidence |
|---|---|
| **The pending query fails in time.** It gives SQLite one bound value for each acknowledged event (`NOT IN (?, ?, …)`). The bundled SQLite accepts at most 32 766. After about 32 700 acknowledgements from one peer (about 3 years at 30 edits a day), that peer is expected to get no more changes. From the code; not reproduced. | `outbox.repository.rs:118-143` |
| **A delete has no LWW check.** A deletes q at t2; B edits q at t3 > t2. On A the edit brings q back; on B the delete removes it. The devices stay different. A received tombstone is never read. | `apply.rs:364-367`; `quest.repository.rs:1165-1176` |
| **The LWW compares with the wrong thing**: the receiver's own newest outbox event, not the stored row. An older change from a third device can win. | `apply.rs:369-374`, `outbox.repository.rs:145-160` |
| **No relay.** A received change does not go into the outbox, and the first sync sends only the device's own events. With pairs A-B and B-C, C never gets A's quests. | `apply.rs:356-506`, `outbox.repository.rs:163-174` |
| **A backup import does not sync.** It records no change. | `src-tauri/src/services/backup.service.rs` (no outbox call) |
| **The row clock is in whole seconds**, and a device with a slow clock loses with an edit that it made after it received the change. | `src-tauri/src/services/db.service.rs:115,129-133` |
| **An alarm still fires on B after A completes or deletes the quest.** After it applies peer changes, SpaceSync publishes only `Event::SpaceDataChanged`, and the reminder service does nothing for that event. | `space-sync.service.rs:1290-1291`; `src-tauri/src/services/reminder.service.rs:300` |

### Constraints

- Sync is whole-row last-writer-wins (LWW) today
  (`SyncEventEnvelope::wins_over`, `src-tauri/src/services/channel.service.rs:716-733`).
  A checklist quest and a checklist day mark merge item by item against a
  base that each device keeps (`checklist_base`; #128; `apply.rs:339-449`,
  `quest.repository.rs:1226-1307`).
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

## Options considered

### Sync approaches

One row for each option. (h) to (m) are NoSQL options (facts below).

| Option | How | Storage growth | Finding differences | Deletes | Edits of different fields at the same time | Relay A-B-C | Privacy (ids in the data) | Sync cost (bytes, round trips, first sync) | Fit with Fini's storage and queries | Complexity | Result |
|---|---|---|---|---|---|---|---|---|---|---|---|
| (a) Outbox + acks, pruning | Today's events; remove an event that all peers acknowledged; drop the queue of a peer away for N days, then a full sync | Bounded by the pruning; acks for each event and peer until then | The queue is the difference | Delete event in the queue | One wins (whole row) | No (defect) | Origin id in each event | 2 fragments + ack for each change | Already there | Low | Not chosen: keeps the outbox (R4) |
| (b) Hash-range state reconciliation, LWW for each row | One set of (version, id) items for each space; negentropy range fingerprints find the different items | Rows + deletion markers (time limit) | O(d · log n); 1 round trip when equal | Markers with LWW | One wins | Yes | None needed | Equal: 1 round trip, ~110 B each way; 1 change: 3–4 round trips, ~3 KB (estimate) | Good; SQL queries stay | Medium | Not chosen: whole-row LWW is ruled out ("Conflict handling") |
| (b′) As (b), a version for each field | The same set; a row merges field by field | Rows + a version for each field + markers | As (b) | Markers | Both stay | Yes | None needed | As (b); smaller rows | Good; more columns or a side table | Medium-high | Not chosen (D1) |
| (c) Sequence numbers for each device | Each write gets the next local number; send "rows above N" for each peer | Rows + markers + one number for each peer | 1 round trip; exact only while the numbers are correct | Markers with numbers | One wins | Yes, if a received row gets a local number | Origin id | Best: 1 round trip + rows | Good; triggers | Low-medium; silent drift after a restore | Not chosen: no self-repair (R3) |
| (d) Automerge | A CRDT document of operations | Full operation history | Bloom filter of heads | Delete operations in the history | Both stay; lists and text merge | Yes | Actor id in each operation | Compact; the history makes the first sync large | Possible as the only store | High | Not chosen |
| (e) cr-sqlite | SQLite extension; CRR tables; `crsql_changes WHERE db_version > N` | Rows + a clock for each column + markers | 1 round trip; numbers | Markers in its clock tables | Both stay (each column) | Yes | Site id of each writer | Small | SQL queries stay; a loadable extension to build for each platform; little activity since 2024 | High (packaging) | Not chosen |
| (f) Git-style | A Merkle tree of the state; commits; for each peer the last common commit; 3-way merge | Rows + one merge base for each peer + short history | Tree hashes from the top: O(d · log n) | No markers while a merge base exists | Both stay (3-way) | Yes | Authors in commits, if stored | Like (b), plus commit data | Medium: base snapshots, commits, a merge step | High | Not chosen |
| (g) Blockchain | A chain of signed blocks with consensus | Grows forever; cannot be pruned | Compare chains | Transactions | — | Yes | Keys of all signers | Large | Poor | Very high | Rejected: no parties that do not trust each other |
| (h) iroh-docs | Signed entries keyed by (author, key); LWW by wall-clock time; range-based set reconciliation | Entries for each author and key; deletion markers are never removed | O(d · log n) | An empty entry for each (author, prefix); it stays | One wins (whole value) | Yes | Author key in each entry | Like (b); each entry carries two signatures; the content is a second transfer (iroh-blobs) | Possible as the only store (redb) | Medium-high | Not chosen |
| (i) Willow / Earthstar | Entries at (subspace, path, time); 3D range-based reconciliation; Meadowcap capabilities | Bounded: a newer entry at a prefix removes older entries below it | O(d · log n) in 3D ranges | Destructive prefix edits | One wins (each path) | Yes | Subspace ids; private overlap detection | Like (b) | Possible as the only store; the Rust implementation is alpha; the sync design is paused | High | Not chosen |
| (j) CouchDB-style | Revision tree for each document; `_changes` since a checkpoint; `_revs_diff` | Revision trees up to `_revs_limit`; tombstones stay | Checkpoint + `_revs_diff` | `_deleted` revision | Losing revisions kept as conflicts | Yes | Revision ids | JSON revisions with history; several round trips | Possible (a young Rust implementation on redb) | High | Not chosen |
| (k) Couchbase Lite | CouchDB-style replication in a C library | As (j) | As (j) | As (j) | As (j) | Yes | As (j) | As (j) | Peer-to-peer only in the Enterprise Edition; the Rust binding is unofficial | High; licence cost | Not chosen |
| (l) Ditto | Closed-source CRDT document store with a mesh | CRDT metadata | Its own protocol | CRDT deletes | Both stay | Yes | Peer ids | Not public | Closed | Low to integrate, high to depend on | Not chosen |
| (m) Generic embedded KV (redb, sled, SurrealDB embedded, RocksDB) | A store only | — | None by itself | — | — | — | — | — | Needs a sync on top | — | Not chosen |

Notes:

- **(b) and "hashes of hashes".** Negentropy (the range-based set
  reconciliation of the Nostr relays) sorts the items by time and compares one
  fingerprint for each range; a range that differs is split into 16. Its
  fingerprint is additive, so it does the job of a Merkle tree with no tree to
  store. Estimates for 2 000 items and 4 KB frames on Bluetooth: 0 differences
  ~220 B (2 fragments), 1 difference ~3 KB (6–8 fragments), 100 recent
  differences ~100–190 KB.
- **(f)** is how today's checklist merge works, in small (`merge_3way`). Prior
  art: Dolt (prolly trees, cell-level 3-way merge), Fossil (the sync
  exchanges the ids of the artifacts that each side has).

### NoSQL options

Checked against the project pages and the source on the day this record was
written.

| | iroh-docs | Willow / Earthstar | CouchDB / PouchDB | Couchbase Lite | Ditto |
|---|---|---|---|---|---|
| **Status** | Released often by the iroh team; uses the same major version of iroh as Fini | Specs active; Rust crates alpha; the Confidential Sync design is paused | Mature | Mature product | Commercial product |
| **Data model** | Namespace → entries keyed by (author, key); an entry holds the hash, size and time of a content blob | Namespace → subspace → path → entry | A JSON document with a revision tree | As CouchDB | JSON documents with CRDT fields |
| **Conflict rule** | For each (author, key): the newer wall-clock time (entries more than 10 minutes ahead are rejected), then the content hash | For each (subspace, path): the newer time, then the digest | A deterministic winner; losers stay as conflicts | As CouchDB | CRDT merge |
| **Deletes and GC** | `del(author, prefix)` writes an empty entry that stays; no GC of markers documented | A newer prefix entry removes the older entries below it | `_deleted` revisions stay; purge does not replicate | As CouchDB | Not public |
| **Privacy (R5)** | The author id is in each entry; one shared author key would hide C | Private interest overlap detection | Revision ids only | As CouchDB | Peer ids in its mesh |
| **Licence** | MIT / Apache-2.0 | MIT / Apache-2.0 | Apache-2.0 | Source Apache-2.0; peer-to-peer and encryption only in the commercial Enterprise Edition | Proprietary; it does not fit Fini's AGPL licence (ADR-0009) without an exception |

Generic KV stores bring no sync. For diffs, only a content-defined Merkle tree
on top matters: a prolly tree (Dolt, noms).

### Fit with Fini's storage and queries

Fini is not tied to SQLite or Diesel. A store must give persistence, the
queries (sorted lists, overdue quests, the Focus pick, the occurrences of a
day, the reminders that are due, the quest relations of ADR-0015), schema
evolution, backup and import, and CLI access.

| | (i) The sync store is the only store | (ii) The sync store is the truth; SQLite is a view | (iii) SQLite stays; a KV sync model over rows |
|---|---|---|---|
| **What changes** | All repositories and queries | Each write goes to the sync store, then into SQLite | Only SpaceSync |
| **Queries** | In memory, on indexes (measured: 2 ms or less for 10 000 quests) | SQL on the view | SQL |
| **Two stores** | No | Yes, with a rebuild after a failure | No |
| **Risk** | Medium | High | Low; it is option (b) |

### Wider survey

Fini has no server, so a server-centric engine is rejected or only a source of
ideas. The transport is not a filter (R7).

| Group | Projects | Result |
|---|---|---|
| Viable | Loro, yrs, p2panda (APIs not stable; author keys in the data), negentropy (the engine of (b)), iroh-docs (h) | Loro chosen (D1); yrs measured below |
| Ideas | TinyBase `MergeableStore` (a version and a hash for each value), rateless IBLT (a small difference in one message), Merkle Search Tree and prolly trees (a stored "hash of hashes"), Evolu (no device ids; the relay sees only owner ids), Actual Budget (HLC and a Merkle trie), Corrosion (a periodic sync as a safety net), Syncthing, Fossil, the SQLite session extension, sqlite-sync (a delete rule for each table), diamond-types, Hypercore | Not used by the decision |
| Rejected | ElectricSQL, PowerSync, Zero, Replicache, InstantDB, Triplit, Convex, WatermelonDB, LiveStore, Jazz, Turso / libSQL, sqlite-sync | A server is the source of truth, or sync goes through a server |
| Rejected | Litestream, LiteFS, rqlite, mvsqlite, Marmot | Server replication, not for offline peers |
| Rejected | Yjs (JavaScript), OrbitDB, Gun, Hypercore / Autobase / Hyperbee, Earthstar, DXOS, RxDB | JavaScript only, logs that grow, or a relay or master |
| Rejected | any-sync, Dolt, TerminusDB, Irmin, Fossil, SurrealDB | Not a Rust library, or no peer-to-peer sync |

### Loro and yrs, measured

A spike outside the repository: a release build in a Linux x86_64 container,
one run each, so the times are approximate. It also compiled for
`aarch64-linux-android`; it did not run on a phone.

Dataset A: 1 000 quests (11 fields, 5 checklist items each), then 10 000
edits, then 300 deletes.

| Measure | Loro | yrs |
|---|---|---|
| Snapshot after the build | 409 KB | 544 KB |
| After the edits and deletes, all history kept | 528 KB | 692 KB (garbage collection off) |
| After the edits and deletes, history removed | 234 KB (shallow snapshot) | 414 KB (garbage collection on) |
| Update for 1 edit / 100 edits | 98 B / 1.6 KB | 15.3 KB / 16.2 KB: each update carries the whole delete set (15.3 KB here) |
| Version vector (state vector) | 5 B | 5 B |
| Catch-up of a peer that was silent through all the edits | 146 KB, 16 ms | 81 KB, 11 ms |

Load and queries, fresh process:

| Measure | Loro, 1 000 quests | Loro, 10 000 quests | yrs, 1 000 quests | yrs, 10 000 quests |
|---|---|---|---|---|
| Snapshot file | 318 KB | 3.3 MB | 544 KB | 5.6 MB |
| Load and first full read | 5 ms | 62 ms | 12 ms | 150 ms |
| Memory of the space doc and the index | +8 MB | +28 MB | +12 MB | +122 MB |
| Build the in-memory index | 3 ms | 45 ms | 3 ms | 44 ms |
| List sort / Focus pick / reminders due | 0.1 / 0.01 / < 0.01 ms | 1.7 / 0.1 / 0.05 ms | 0.1 / 0.01 / < 0.01 ms | 1.6 / 0.08 / 0.04 ms |
| Size added to the Android binary (stripped) | +3.8 MB | | +0.6 MB | |

On a mid-range phone, the estimate is the same memory and two to four times
the Linux times (estimate: 120–250 ms to start with 10 000 quests in Loro).

Other measured facts:

- A Loro peer older than the point of a shallow snapshot gets
  `ImportUpdatesThatDependsOnOutdatedVersion` for its own edits.
- After relay A-B-C, A's version vector holds C's peer id (both libraries).
- Concurrent Loro tree moves A under B and B under A: both devices end with A
  under B and B at the root.
- yrs payloads can be made small with Fini's own protocol (blocks only, a hash
  of the delete set, the missing delete-set ranges on a difference): 10 B
  with no change, 29 B for 1 edit, 1.4 KB for 100. v2 encoding with zstd
  gives 2.4–3.3 KB. yrs has no history, tree or counter, and its newest
  release needs a newer Rust than Fini's build images.

| | Loro | yrs |
|---|---|---|
| Licence | MIT | MIT |
| Interface | Stable; releases several times a month | Releases that can change the interface every one to three months |
| Main authors | One main author and one second | One main author |

### The candidates against the conflict policy

The policy ("Conflict handling") needs a merge for each field and a bounded
history of each quest.

| Candidate | Merge for each field | History of a quest | Counter as a sum |
|---|---|---|---|
| (a), (b), (c) (whole row) | ✗ | ✗ | ✗ |
| (b′) | ✓ | A table of its own | A value for each device |
| (d) Automerge | ✓ | ✓, with no bound | ✓ |
| (e) cr-sqlite | ✓ (each column) | ✗ | ✗ |
| (f) Git-style | ✓ (3-way) | ✓ the commits | A value for each device |
| (h) iroh-docs | ✓ with one key for each field | The last entry of each author only | A key for each author |
| (j), (k) CouchDB-style | ✗ (whole document) | The losing revisions, until compaction | ✗ |
| Loro | ✓ (each map key) | ✓ in its history, bounded by shallow snapshots | ✓ |
| yrs | ✓ (each map key) | ✗ (garbage collection) | ✗ |

### Loro or (b′)

(b′) is Fini's own design: a version for each field, a history table, and
hash-range reconciliation.

| | Loro | (b′) |
|---|---|---|
| **Peer ids in the data** | Yes: A can see that a third writer exists, not who (D10) | None |
| **Sync payload** | 98 B for 1 edit, 1.6 KB for 100 (measured) | 3–4 round trips, about 3 KB for 1 edit (estimate) |
| **Storage** | The space doc is the store | SQLite stays, with field versions and a history table |
| **Tree, counters, list moves** | Built in | Written by Fini |
| **Build effort** | Storage, queries and CLI access written again; the sync comes with the library | The reconciliation, versions, history and rules written by Fini |

### Trust models

| | Authenticated iroh links only | Signed rows, original signer | Signed rows, signed again at each hop |
|---|---|---|---|
| **Proves who wrote a row** | No; only which paired device sent it | Yes | No; only the last hop |
| **Hides C from A (R5)** | Yes, if the rows hold no device id | No: A needs C's key | Yes |
| **Cost** | None | About 100 B and one verify (estimate 50–150 µs on a phone) for each row | The same, at each hop |
| **Digests** | No effect | The signature can be in the digest | The signature must stay out of the digest |

### Clocks and deletion markers

These apply only to the options that are not chosen. A hybrid logical clock
(HLC) gives the larger of the device time and the newest version seen; it was
chosen before Loro, then dropped (D5). Deletion markers with a 90-day limit
for silent peers apply to (b), (b′), (c) and (e).

## Decision

**Guiding principle: keep the Loro integration thin.** Use what Loro does by
itself wherever possible, so that the migration has little custom code that
can break. A custom solution is not rejected, but it is deferred, to keep the
surface for bugs small during the migration.

| Item | Loro's own (now) | Custom (deferred, optional) |
|---|---|---|
| D3 Sync | Version vectors, export of updates, import | — |
| D6 History | Lost values from Loro's history (checkout, diff) | An explicit list of the last 10 values of each field |
| D7 Shallow snapshots | None during the migration | Shallow snapshots, and the re-apply path for an old peer |
| D8 Descriptions | A plain child text | A merge of an edit made before the first comparison |
| D9 Repeating quests | The rule as one value; marks as mergeable containers | Archiving old closed marks |
| D11 CLI | Import of each update file | The compaction rule for files; the write lock |
| Migration | Mergeable containers | — |

### D1 — Loro is the sync engine and the only store

SpaceSync syncs space docs, and the outbox goes (R4). There is no SQLite: the
data of each space is a snapshot and update files (see "Data model"). (b′) is
not chosen because Fini would write the reconciliation, the field versions,
the history and the rules itself; yrs is not chosen because it needs Fini's
own delete-set protocol and has no history, tree or counter.

### D2 — One space doc for each space

A space is the unit that a pair maps. A move of a quest to another space is a
soft delete in one space doc and a create in the other.

### D3 — Sync

Over the iroh session (`fini/sync/7`), each device sends its version vector
for each mapped space, and each side sends the edits that the other lacks
(measured: 98 B for one edit, 1.6 KB for 100). A comparison runs after each
local edit (after about 2 s with no more edits) and every 30 s while
connected; there is no other push. Relay A-B-C works, because B's space doc
holds A's edits.

Loro is collaborative editing that does not need a connection. Each device
commits its edits locally at once; there are no transactions across devices
and no locks. A sync at any time, after seconds or after days, merges the two
space docs, and the result is the same on all devices.

### D4 — Soft deletes

A delete is a soft delete: it sets `deleted_at` and keeps the map key.
"Recently deleted" lists these quests, and a restore clears `deleted_at`. A
soft delete and a concurrent edit both stay, because they are two fields.

### D5 — Concurrent writes of one field

Loro's own order: the higher Lamport time wins, then the higher peer id. An
edit made after a device saw another edit wins over it; for concurrent edits
the order is deterministic, and all devices converge. There is no HLC.

### D6 — The conflict history comes from Loro's history

The app reads the values that lost from Loro's history (a checkout of an
earlier version, or a diff), shows up to the last 10 values of a field, and a
restore writes the chosen value as a new edit. The history goes back to the
point of a shallow snapshot; with none during the migration (D7), it is
complete.

### D7 — No shallow snapshots during the migration

The snapshot keeps the full history (measured: 528 KB against 234 KB for
1 000 quests after 10 000 edits), so no peer is ever older than the point of a
shallow snapshot. Deferred: shallow snapshots only at a point that all active
peers have passed, with the 90-day rule for silent peers, and a returning old
peer that writes its local changes again as new edits.

### D8 — Descriptions and checklists

The description is a Loro text, a plain child text made when the quest is
made or migrated: edits on different lines from two devices both stay. A
checklist is Markdown task lines in the description (`- [ ] item`,
`- [x] item`), edited only in the editor; its progress is counted from the
text, and an item has no identity. #128's item merge and `checklist_activity`
go away. A title is a field (D5).

### D9 — Repeating quests

The repeat rule is one value of the quest map; `due` is a separate field, and
both follow D5. A concurrent rule change and date change can pair one
device's rule with the other device's date: an accepted edge case. Each
occurrence mark is a mergeable container keyed `<quest_id>/<YYYY-MM-DD>`,
which stores only the overrides that differ from the quest. A rule change
keeps past marks; the open days come again from the new rule, from today.

### D10 — Peer ids

Each installation makes, once at its first start, one id for the app and one
for the CLI: a UUID with a random 64-bit peer id. The ids have no relation to
the iroh key or the device id, and the UI never shows them. A store that is
restored or copied to another device makes both ids again. Accepted
trade-off: a device can see that another writer exists, but not who, which
relaxes R5.

### D11 — The CLI is a Loro peer

Each writer writes its edits to an update file of its own, under a temporary
name, then renames it into place. A reader imports all complete update files
and gets no lock. A CLI write gets an exclusive operating-system file lock on
the CLI peer id, waits for it with no error (a long timeout is only a safety
net), and the system releases the lock when a process stops. The app never
waits for the CLI, and only the app compacts.

### D12 — Trust

Authenticated iroh links only: no signature for each row. Signatures can come
later if a relay that is not trusted appears. An unpair is local to this
device, and the data that the unpaired device wrote stays.

### D13 — Local to each device

Reminders, snoozes, notifications, settings and focus history do not sync.
Each device derives its reminders from the synced quest, rule and marks, and
after each import computes them again and schedules or cancels its own
notifications. This removes the alarm defect ("Defects").

### D14 — What syncs

Quests with their descriptions, repeat rules and occurrence marks, space
names, and imported backups (an import is an ordinary edit).

### D15 — Quest relations

Quest relations are decided in ADR-0015; the engine choice does not preset
that model. The order of the quests stays the rank keys, a field of each
quest.

### D16 — Rust

Loro builds with the Rust of Fini's build images (measured). No toolchain
change is needed.

## Conflict handling

Principle: **Merge automatically and deterministically, and never lose data
silently; history replaces prompts.**

| Situation | Example | Handling |
|---|---|---|
| Different fields of one quest | The phone renames "Gym"; the PC moves its due date | Both are kept |
| The same simple field (status, due date, priority, energy, rank) | Both devices change the priority | One change wins, the same on all devices (D5). The losing value is in the history (D6). No prompt |
| The same text field | Both devices edit the description | Edits on different lines both stay (D8). A title is a short field: one change wins, and the other is in the history |
| Delete against edit | The phone deletes the quest; the PC edits it | The newer action wins. A delete that wins moves the quest to "recently deleted", where the person can restore it |
| Complete against edit | The phone completes the quest; the PC moves its date | Both are kept: they are different fields |
| Both add checklist items | One adds "eggs", the other adds "milk" | Both lines stay (D8) |
| Both reorder | Both devices move the same quest in the list | The rank of the quest: one change wins (D5) |
| Tree moves that make a cycle | One device moves A under B; the other moves B under A | A deterministic rule removes the cycle, the same on all devices (relations: ADR-0015) |
| The same repeating day completed twice | The phone and the PC both complete Monday of "Gym" | One completed mark (D9) |
| Counters | Both devices add 25 focus minutes | Sum: 50 minutes |

The history keeps the last 10 losing values of each field.

## Data model

### Files

```
<data dir>/spaces/<space-id>/
├─ snapshot.bin              the snapshot (only the app writes it)
└─ updates/
   ├─ <peer-id>-<n>.bin      an update file of one writer
   └─ ...
<data dir>/local/            data of this device only (settings, pairs,
                             channels, mappings, reminders, snoozes,
                             focus history); it never syncs
```

The data is a tree of containers: maps, texts and counters. Relations are by
id, as in a NoSQL store, with no foreign keys and no joins. The app code
checks integrity: a dangling reference is ignored when it is read. In-memory
indexes are built at load (measured: 45 ms for 10 000 quests) and kept up to
date from Loro's events.

### A space doc

```
space doc "<space-id>"
├─ meta: Map              name, created_at
├─ quests: Map            quest_id → Map (mergeable container)
│   ├─ title, status, energy, priority, due, due_time, completed_at,
│   │  order_rank, created_at, updated_at, deleted_at, space_id
│   ├─ repeat_rule: one value   { preset, interval, unit, days_of_week,
│   │                             end, end_date, end_after }
│   ├─ description: Text  (a plain child text; checklist lines are in it)
│   ├─ focus_enter_count  (the migrated count, a plain field)
│   └─ focus_enters: Counter   (new enters only)
├─ occurrences: Map       "<quest_id>/<YYYY-MM-DD>" → Map (mergeable container)
│                         status, closed_at, title?, description? (Text),
│                         energy?, priority?, due_time?, created_at,
│                         updated_at
└─ (quest relations: ADR-0015)
```

### Today's fields

The repeat rule today is a JSON text (`RepeatRule`,
`src-tauri/src/models/quest.rs:372-413`). The marks are `quest_occurrences`
(`src-tauri/migrations/00000000000028_quest_occurrences/up.sql:11-24`,
`src-tauri/src/models/quest_occurrence.rs:20-37`); the code that writes them is
`close_occurrence` (`quest.repository.rs:956-985`), `complete_through_today`
(`:1086-1116`), `reopen_occurrence` (`:988-1019`), the "This day" and "All
days" edits (`:845-899`) and `stop_repeat` (`:1024-1082`).

| Table | Field | In the space doc | Derived | Local to the device | Removed |
|---|---|---|---|---|---|
| `quests` | `id` | the map key | | | |
| | `space_id`, `title`, `status`, `energy`, `priority`, `due`, `due_time`, `completed_at`, `order_rank`, `created_at`, `updated_at` | ✓ fields | | | |
| | `description` | ✓ a Text | | | |
| | `repeat_rule` | ✓ one value | | | |
| | `focus_enter_count` | ✓ the migrated count; new enters in a Counter | | | |
| | `is_checklist` | | from the text | | |
| | `checklist_base` | | | | ✓ |
| | (new) `deleted_at` | ✓ | | | |
| `quest_occurrences` | `quest_id`, `period_key` | the key `"<quest_id>/<YYYY-MM-DD>"` | | | |
| | `status`, `closed_at`, `title`, `energy`, `priority`, `due_time`, `created_at`, `updated_at` | ✓ fields | | | |
| | `description` | ✓ a Text (the day copy) | | | |
| | `checklist_base` | | | | ✓ |
| `reminders` | `id`, `quest_id`, `type`, `mm_offset`, `due_at_utc`, `created_at`, `scheduled_notification_id` | | the reminder of a dated quest | ✓ custom rows | |
| `notification_snoozes` | `reminder_id`, `fire_at_utc`, `created_at` | | | ✓ | |
| `focus_history` | `id`, `quest_id`, `space_id`, `trigger`, `created_at` | | | ✓ | |
| `checklist_activity` | all | | | | ✓ |

Derived, never stored: the dates of the rule, the open occurrences, the
oldest open one, the day as the person sees it (`RepeatingQuest::on_day`,
`quest.rs:647-681`), the remaining count of `after_n`, and the History list.

### Growth

One mark for each closed or changed day of each repeating quest: measured
73 B a mark with its history, 37 B in a shallow snapshot. A daily quest adds
about 27 KB a year, 20 daily quests about 530 KB (estimate). A rule is about
100 B, plus about 100 B of history for each change (estimate). This needs no
trim for years; archiving old closed marks is deferred.

## Consequences

- No table grows with each change or each peer (R1, R4). The full history
  stays in the snapshot during the migration (D7).
- Finding the difference costs one round trip and a few bytes (R2, R7).
- Relay, deletes, backup import and reminders behave the same on all devices;
  the defects of "Context" go.
- The data holds peer ids (D10): a device can see that another writer exists.
- SQLite, Diesel queries and `OutboxRepository` go; the queries run on
  in-memory indexes.
- The Android binary is about 3.8 MB larger, and a space with 10 000 quests
  uses about 28 MB of memory (measured).
- The Device page sync queue reads the version vectors, not the outbox.

## Migration and rollout

1. **First start of the new version.** The installation makes its app and CLI
   peer ids (D10).
2. **Space docs from SQLite.** For each mapped space, each device writes its
   rows into a space doc:
   - each quest and each occurrence mark as a mergeable container, so two
     devices that migrate the same rows merge with no duplicates;
   - the repeat rule as one value;
   - the description as a plain child text, with today's checklist items as
     `- [ ]` and `- [x]` lines;
   - `focus_enter_count` as a plain field (a counter would double).
3. **Sync.** `fini/sync/7` replaces `fini/sync/6`. Devices on the two versions
   do not connect, and the Device page shows a notice.
4. **Local data.** Reminders, snoozes, settings and focus history move to
   `<data dir>/local/`.
5. **Removal.** The SQLite file, with the outbox tables, stays unused for one
   release; then the app removes it.

Measured for step 2 (two devices with the same data; after the migration,
each device edits; then the merge):

| What | How | Result |
|---|---|---|
| 1 000 quests | `insert_container` | The edits of one device are lost; the snapshot doubles (208 KB → 403 KB) |
| 1 000 quests | mergeable containers | Both edits kept (250 KB → 344 KB) |
| Checklist items | a mergeable list | 10 items in place of 5: rejected; the checklist is text |
| Counters | a mergeable counter | 50 in place of 25: rejected; the migrated count is a plain field |
| Descriptions | a mergeable text | The text two times: rejected |
| Descriptions | a plain child text (chosen) | The text once; an edit made before the first merge stays readable in the losing child, and the history shows it |
| 50 repeat rules, 1 500 marks | the rule as one value; marks as mergeable containers | No duplicates; the same space doc on both |
| Repeat rules | a mergeable map of fields | Two concurrent edits make a rule that no device wrote: rejected |
| 3 650 marks | mergeable containers | No duplicates; both edits of one day kept (262 KB, 73 B a mark) |
| 2 000 CLI runs | one CLI peer id | 9 B of version vector; a new id for each run gave 8.0 KB, which a shallow snapshot does not remove |

## Test strategy

- **Two-device convergence property test**: random edits, soft deletes,
  restores, rule changes and day marks on two space docs, then the merge; the
  two deep values must be equal, and a second sync must send nothing. Also
  with three devices in a line (relay).
- **Migration tests**: two devices migrate the same rows; no duplicate quest,
  mark, checklist line or rule, and the counts are not doubled (the measured
  cases above).
- **CLI tests**: two CLI writes at the same time write one after the other; a reader gets no
  lock; a stopped process releases the lock; the app imports a new update
  file.
- **Reminder tests**: an import that completes or soft-deletes a quest
  cancels its notification.
- **Multi-device e2e** (`specs/e2e/multi-device/tests/`): update
  `personal-space-sync.spec.ts`, `zz-personal-space-live-quest-sync.spec.ts`,
  `peer-sync-over-ble.spec.ts` and `device-page.spec.ts` (the sync queue), and
  `specs/e2e/feature-plane-cli.spec.ts`, which reads `outbox_event_count`.

## Discussion log

| Topic | Status | Reason |
|---|---|---|
| Remove the outbox | Chosen | It is the cause of the growth (R1, R4) |
| Merkle tree, "hashes of hashes" | Liked; not used | Loro's version vectors find the difference in one round trip |
| Loro or (b′) | Chosen: Loro (D1) | The project owner accepts the peer ids (D10) |
| yrs | Not chosen | Each update carries the whole delete set (15.3 KB measured); smaller only with Fini's own protocol; no history, tree or counter |
| Git-style merge base (f) | Liked; not chosen | Deletes with no markers, but a base for each peer |
| Automerge (d) | Not chosen | The history grows; actor ids in each operation |
| iroh-docs, Willow, CouchDB-style, Couchbase Lite, Ditto, generic KV | Not chosen | See "NoSQL options" |
| Blockchain (g) | Rejected | No parties that do not trust each other; an append-only log cannot be pruned |
| Server-centric engines | Rejected | Fini has no server |
| Fini tied to SQLite | Rejected | A sync store can be the only store |
| Thin Loro integration | Chosen | Little custom code during the migration |
| Merge unit | Chosen: each field | Whole-row LWW is ruled out |
| HLC | Chosen before Loro; dropped (D5) | Loro's order also lets a later edit win, and all devices converge |
| Versions more than 24 h ahead rejected | Chosen before Loro; not used | Loro's order uses no wall clock |
| Deletion markers, 90-day rule | Chosen for the markers; not used | Loro uses soft deletes (D4); the 90-day rule applies to deferred shallow snapshots (D7) |
| History bound | Chosen: the last 10 losing values of each field | A fixed bound |
| Explicit 10-value list | Deferred | Loro's history gives the values while there is no shallow snapshot |
| Long descriptions | Changed: a Loro text (D8) | The earlier choice "newer wins, with history" was replaced |
| Checklists | Chosen: lines of the description (D8) | No checklist entity |
| Description migration | Chosen by measurement: a plain child text | A mergeable text doubles the text; a first-edit text doubles it when two devices edit first; a fixed migration peer id makes the devices different when the data differs; "the smaller id builds" needs a rule that fails with two local minimums |
| Repeat rule | Chosen by measurement: one value (D9) | A mergeable map mixes two concurrent edits |
| Rule and `due` at the same time | Accepted edge case (D9) | Not important |
| Occurrence marks | Chosen: mergeable containers (D9) | No duplicates; both edits of a day kept |
| Trust | Chosen: authenticated iroh links only (D12) | Row signatures and signing again at each hop: not chosen |
| Peer ids | Chosen: one app id and one CLI id for each installation (D10) | A new id for each run grows the version vector; a pool of 4 CLI ids is not necessary |
| CLI write lock | Chosen (D11) | Only writes get it; reads get none |
| Live push | Rejected | Comparisons only (D3) |
| Reminders, snoozes, notifications | Chosen: local (D13) | Derived again after each import |
| Focus history | Chosen: local (D13) | Each device has its own focus, because the spaces differ for each device |
| Sync scope | Chosen (D14) | Space names and imported backups sync; settings stay local |
| Unpair | Chosen: local (D12) | The data of the unpaired device stays |
| Version mismatch | Chosen: a notice on the Device page | Migration step 3 |
| Old outbox tables | Chosen: unused for one release, then removed | Migration step 5 |
| Quest relations | ADR-0015 (D15) | The engine choice does not preset the model |
| A newer Rust | Not needed for Loro (D16) | — |
