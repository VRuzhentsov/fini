# 0014 — Sync by state: the options

## Status

Proposed. This record compares options; it gives no decision yet. It was
written during the design discussion of 2026-11-24, and it records what the
project owner liked, rejected and left open. It answers the research part of
issue #196.

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

Status: chosen by the project owner, except the items marked open.

Principle: **Merge automatically and deterministically, and never lose data
silently; history replaces prompts.**

| Situation | Example | Handling |
|---|---|---|
| Different fields of one quest | The phone renames "Gym"; the PC moves its due date | Both are kept: a merge for each field |
| The same simple field (status, due date, priority, energy, rank) | Both devices change the priority | The newer change by HLC wins, the same on all devices. The losing value goes into the history of the quest, so the person can restore it. No prompt |
| The same text field | Both devices edit the description | The newer change wins, and the other goes into the history (also for titles). A text-CRDT merge for long descriptions: open |
| Delete against edit | The phone deletes the quest; the PC edits it | The newer action wins. A delete that wins moves the quest to "recently deleted", where the person can restore it |
| Complete against edit | The phone completes the quest; the PC moves its date | Both are kept: they are different fields |
| Both add checklist items | One adds "eggs", the other adds "milk" | Union: both items stay |
| Both reorder | Both devices move the same quest in the list | The rank of each quest: the newer change wins |
| Tree moves that make a cycle | One device moves A under B; the other moves B under A | A deterministic rule removes the cycle, the same on all devices |
| The same repeating day completed twice | The phone and the PC both complete Monday of "Gym" | Idempotent: one completed mark |
| Counters | Both devices add 25 focus minutes | Sum: 50 minutes |

Open: how the history of a quest is bounded (for example the last N values
of each field, or a time limit).

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

## Sync approaches

One row for each option. Rows (h) to (m) are the NoSQL options; see
"NoSQL options" below for their facts.

| Option | How | Storage growth | Finding differences | Deletes | Edits of different fields at the same time | Relay A-B-C | Privacy (device ids in the data) | Sync cost (bytes, round trips, first sync) | Fit with SQLite and Diesel | Complexity | Status |
|---|---|---|---|---|---|---|---|---|---|---|---|
| (a) Outbox + acks, pruning | Today's events; remove an event that all peers acknowledged; drop the queue of a peer away for N days, then a full sync | Bounded by the pruning; acks for each event and peer until then | The queue is the difference | Delete event in the queue | One wins (whole row) | No (defect) | Origin id in each event | 2 fragments + ack for each change | Already there | Low | Not chosen: keeps the outbox (R4) |
| (b) Hash-range state reconciliation, LWW for each row | One set of (version, id) items for each space; negentropy range fingerprints find the different items; send those rows | Rows + deletion markers (time limit) | O(d · log n); 1 round trip when equal | Markers with LWW; a time limit for silent peers | One wins; checklists merge as today | Yes: B's set has A's rows | None needed: the tie rule can use a content hash | Equal: 1 round trip, ~110 B each way; 1 change: 3–4 round trips, ~3 KB | Good: SQL tables, a crate for the protocol | Medium | Liked: "hashes of hashes", outbox removed |
| (b′) As (b), a version for each field | The same set; a row merges field by field | Rows + a version for each field + markers | As (b) | Markers | Both stay | Yes | No id needed | As (b); smaller rows | Good; more columns or a side table | Medium-high | Open |
| (c) Sequence numbers for each device | Each write gets the next local number; send "rows above N" for each peer | Rows + markers + one number for each peer | 1 round trip; exact only while the numbers are correct | Markers with numbers | One wins | Yes, if a received row gets a local number | Origin id to stop echo | Best: 1 round trip + rows | Good; triggers | Low-medium; silent drift after a restore | Not chosen alone: no self-repair (R3); a fast path later |
| (d) Automerge | A CRDT document of operations; sync messages for each peer | Full operation history | Bloom filter of heads | Delete operations in the history | Both stay; lists and text merge | Yes | Actor id in each operation | Compact; history makes the first sync large | Poor: a second data store | High | Discussed; not chosen |
| (e) cr-sqlite | SQLite extension; CRR tables; `crsql_changes WHERE db_version > N` | Rows + a clock for each column + markers | 1 round trip; numbers | Markers in its clock tables | Both stay (each column) | Yes | Site id of each writer | Small | Poor: a loadable extension that rewrites the tables; little activity since 2024 | High (packaging) | Not chosen |
| (f) Git-style | A Merkle tree of the state; commits; for each peer the last common commit; 3-way merge | Rows + one merge base for each peer + short history | Compare tree hashes from the top: O(d · log n) | No markers: in the base and missing on one side is a delete | Both stay (3-way) | Yes: B's commits hold the merge with C | Author in commits if stored; merge commits show C's hashes unless squashed | Like (b), plus commit data | Medium: base snapshots, commits, a merge step | High | Liked, under discussion |
| (g) Blockchain | A chain of signed blocks with consensus | Grows forever; cannot be pruned | Compare chains | Transactions | — | Yes | Keys of all signers | Large | Poor | Very high | Rejected |
| (h) iroh-docs | Signed entries in a namespace, keyed by (author, key); LWW for each (author, key) by wall-clock time; range-based set reconciliation | Entries for each author and key; deletion markers are never removed | O(d · log n) | Prefix delete: an empty entry for each (author, prefix); it stays | One wins (whole value) | Yes | Author key in each entry; one shared author key for all devices would hide C | Like (b) for the entries; each entry carries two signatures and the ids; the content is a second transfer (iroh-blobs) | Poor: a second store (redb), content as blobs | Medium-high | Under discussion |
| (i) Willow / Earthstar | Entries at (subspace, path, time); 3D range-based reconciliation; Meadowcap capabilities | Bounded: a newer entry at a prefix removes older entries below it, with no marker | O(d · log n) in 3D ranges | Destructive prefix edits | One wins (each path) | Yes | Subspace (author) ids in entries; private overlap detection hides what is not shared | Like (b), in 3D ranges; capabilities add bytes at the start of a session | Poor: its own store; Rust implementation in alpha, sync protocol design paused | High | Under discussion |
| (j) CouchDB-style | Revision tree for each document; `_changes` since a checkpoint for each peer; `_revs_diff` | Revision tree for each document up to `_revs_limit`; tombstones stay; purge does not replicate | Checkpoint + `_revs_diff` | `_deleted` revision (tombstone) | Losing revisions kept as conflicts; app merges | Yes | Revision ids; no device id needed | JSON revisions with their history; several round trips for each batch; the first sync sends the revision trees | Poor: no SQLite-backed Rust implementation | High | Under discussion |
| (k) Couchbase Lite | CouchDB-style replication in a C library | As (j) | As (j) | As (j) | As (j) | Yes | As (j) | As (j) | Poor: its own SQLite file; peer-to-peer only in the Enterprise Edition; Rust binding unofficial | High; licence cost | Under discussion |
| (l) Ditto | Closed-source CRDT document store with a mesh over Bluetooth, LAN and Wi-Fi | CRDT metadata in its store | Its own protocol | CRDT deletes | Both stay (CRDT) | Yes (mesh) | Peer ids in its mesh | Not public | Poor: a second, closed store | Low to integrate, high to depend on | Under discussion |
| (m) Generic embedded KV (redb, sled, SurrealDB embedded, RocksDB) | A store only | — | None; only a prolly tree on top gives a diff | — | — | — | — | — | Poor: replaces SQL | — | Under discussion |

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

  A live push of a changed row while a session runs (one frame, no
  acknowledgement) keeps one edit at two fragments. The next run confirms it.
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

### Fit with Fini's data

Fini's data is relational: quests, occurrence marks, reminders, checklist
items, spaces, rank keys, and soon a parent tree with recursive queries
(ADR-0015). Diesel reads it with SQL and indexes.

| | (i) A NoSQL store is the source of truth | (ii) KV replication is the truth; SQLite is a view built from it | (iii) SQLite stays; the sync uses a KV model over rows (key = table/id) |
|---|---|---|---|
| **What changes** | All repositories, queries and indexes are written again on the new store | Each write goes to the KV store; a step copies it into SQLite tables for the queries | Only SpaceSync: each synced row is a (key, version, value) entry |
| **Queries (lists, Focus, the parent tree)** | Written again by hand, with no SQL | SQL, on the view | SQL, as today |
| **Two stores** | No | Yes: two stores to keep the same, and a rebuild after a failure | No |
| **Migrations** | All data moves | All data moves; SQLite migrations stay | Columns only |
| **Sync** | The store's own (h)–(l) | The store's own | Option (b) or (b′): the same as this model |
| **Risk** | Highest | High | Lowest |

Option (iii) is the KV sync model without a second store, and it gives the
same design as (b). iroh-docs and Willow are useful as references for (iii):
range-based reconciliation, signed entries, prefix deletes.

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

## Trust and integrity

Status: undecided. The project owner is unsure about re-signing.

| | Authenticated iroh links only (no signature for each row) | Signed rows, original signer | Signed rows, signed again at each hop |
|---|---|---|---|
| **How** | The TLS handshake proves the paired peer on each link (today). Rows carry no signature. | The device that wrote a version signs it. A relay sends the signature unchanged. | Each device signs the rows that it sends with its own key. |
| **Proves who wrote the row** | No. It proves only which paired device sent it. | Yes, through any number of hops. | No. It proves only the last hop, which the link already proves. |
| **Hides C from A (R5)** | Yes, if the rows hold no device id. | No. A sees C's key, and needs it to verify (C must be introduced to A). | Yes. |
| **Storage and CPU** | None. | About 100 B for each row (signature and key). Ed25519 verify about 50–150 µs on a phone; a first sync of 10 000 rows about 0.5–1.5 s (estimates). | The same storage. A sign and a verify at each hop. |
| **Relay and digests** | No effect. | The signature is the same on all devices, so it can be in the item id or out of it. | The signature changes at each hop, so it must stay out of the item id and the digests. If not, each hop looks like a difference. |
| **Revocation at unpair** | Stop the link (today). | The revoked key's rows must be kept or signed again by another device. | Stop the link. Stored signatures of the revoked device stay as history only. |

Candidate from the earlier draft (not decided): sign with the iroh key and a
domain tag, keep the original signer through relay, and sign again the rows
of a device when it is unpaired. It meets "proves who wrote the row", but it
fails R5.

## Version clock

| | Wall clock (today) | Hybrid logical clock (HLC) |
|---|---|---|
| **What it is** | The device time of the write | The larger of the device time and the newest version that the device has seen, with a counter for the same millisecond (Kulkarni et al.; CockroachDB, Actual Budget) |
| **An edit after a received change** | Can lose if the clock is slow | Always wins |
| **Two writes in the same millisecond** | Same version | Different versions |
| **A device with a clock far ahead** | Its edits win until the time catches up | It pulls the other clocks forward; the order stays correct |
| **Reads as a time** | Yes | Yes, for devices whose clocks agree |
| **Status** | — | Favored |

Chosen so far: a row whose version is more than 24 hours ahead of the
receiver's clock is rejected, and the Device page shows a warning about the
clock. The tie rule for two equal versions must not need a device id (R5):
for example the content hash.

## Deletion markers and silent peers

These apply to (b), (b′), (c) and (e). Option (f) needs no markers while a
merge base exists.

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
| HLC | Favored | An edit after a received change always wins |
| Trust model | Open | Signatures against privacy (R5) |
| Relay signed again at each hop | Open | The project owner is unsure; it proves no more than the link does |
| Merge unit: row, field, or 3-way with a base | Chosen: a merge for each field | Whole-row LWW is ruled out (see "Conflict handling") |
| Conflict handling policy | Chosen | Merge automatically and deterministically; never lose data silently; history replaces prompts |
| History bound of a quest | Open | The last N values, or a time limit |
| Text-CRDT merge for long descriptions | Open | Newer wins with history is the default |
| iroh-docs (h) | Under discussion | Same stack as Fini, and the same reconciliation; but author ids in entries (R5), markers that stay, and a second store |
| Willow / Earthstar (i) | Under discussion | Prefix deletes with no marker growth and private overlap detection; the Rust code is alpha and its sync design is paused |
| CouchDB-style (j) | Under discussion | Mature model; revision trees and tombstones grow; no SQLite-backed Rust implementation |
| Couchbase Lite (k) | Under discussion | Peer-to-peer only in the commercial edition; unofficial Rust binding |
| Ditto (l) | Under discussion | A finished mesh product; closed source, paid tiers, conflicts with AGPL |
| Loro | Under discussion | Rust CRDT with a movable tree (the parent tree of ADR-0015) and text; history grows unless trimmed by shallow snapshots |
| yrs | Under discussion | Rust CRDT for text fields such as descriptions and checklists; not for relational rows |
| p2panda | Under discussion | Rust, on iroh; but its APIs are not stable, and author keys are in the data (R5) |
| Rateless IBLT | Under discussion | One message for a small difference, which suits a slow link |
| A stored Merkle tree (Merkle Search Tree, prolly tree) | Under discussion | The "hashes of hashes" in a stored form |
| A version and a hash for each value (TinyBase) | Under discussion | A model for (b′) |
| Server-centric engines | Rejected | Fini has no server |
| Generic KV (m) and the fit analysis | Under discussion | No sync by itself; a NoSQL source of truth (i) or a second store (ii) costs all queries; (iii) is (b) |

## Open questions

1. Which sync approach: (b), (b′) or (f)? Or (b) now and (f) later?
2. The history of a quest (see "Conflict handling"): keep the last N values
   of each field, or keep values for a time? And should long descriptions
   merge as text (a text CRDT)?
3. Which trust model: links only, signed rows with the original signer, or
   signed at each hop? Is "proves who wrote the row" a requirement, given R5?
4. For (f): how much history to keep, and where to keep the merge base of
   each peer (a full snapshot, or shared parts once)?
5. Should the data keep device ids at all (for example
   `checklist_activity.origin_device_id`), given R5?
6. With markers: is the candidate removal rule and its two edge cases
   acceptable?
7. Should `focus_history` and the space names sync?
8. Should a backup import sync at once, or stay local until the person edits
   the imported quests?
9. The protocol: `fini/sync/7` with a notice on the Device page when the peer
   runs another version. Keep the old sync tables for one release so that a
   rollback is possible?
10. NoSQL: keep SQLite and use the KV sync model over rows (iii), or test a
    NoSQL store (iroh-docs or Willow) as the sync store in a spike first?
11. If iroh-docs is a candidate: one author key for all devices of the person
    (hides C, R5), or one author for each device (shows who wrote)?
