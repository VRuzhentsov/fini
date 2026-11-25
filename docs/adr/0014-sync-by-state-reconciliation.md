# 0014 — SpaceSync reconciles state, and each change is signed

## Status

Proposed. Written during the design discussion of 2026-11-24. The project
owner chose state reconciliation with digest trees and signed changes, and
the removal of the outbox. Nothing in this record is implemented yet. It
answers issue #196. When it is accepted, it supersedes the kept frames of
ADR-0008 D14, the outbox part of ADR-0012 D11, and rule 6 of
[`db.md`](../db.md) (the transactional outbox).

Paths are relative to the repository root. A line number points at the code
of the main branch on the day this record was written.

## Context

### What SpaceSync stores today

SpaceSync records each local change as one event and keeps the event:

| Table | Where | What one row is |
|---|---|---|
| `sync_outbox` | `src-tauri/migrations/00000000000010_sync_and_focus/up.sql:35-49` | one change that this device made: the whole object as JSON, two UUIDs, `updated_at` |
| `sync_acks` | same file, `:52-57` | one change that one peer acknowledged. It has no foreign key to `paired_devices`, so an unpair leaves its rows |
| `sync_seen` | same file, `:60-63` | one change from a peer that this device applied |
| `tombstones` | same file, `:66-74` | one delete that this device received |
| `peer_control_outbox` | `src-tauri/migrations/00000000000026_peer_control_outbox/up.sql:9-17` | one control frame (`SpaceSyncEnd`) that waits for a session |

The models are in `src-tauri/src/models/sync.rs:6-84`. All reads and writes
are in `OutboxRepository` (`src-tauri/src/repositories/outbox.repository.rs`).

Each write of a quest writes the full stored row (`record_quest`,
`outbox.repository.rs:317-368`). A move to another space writes a delete, an
upsert and one event for each occurrence mark. A rank move writes one quest,
and a rebalance writes all quests of the space (`:372-383`,
`src-tauri/src/repositories/quest.repository.rs:494-557`). The callers are
in `src-tauri/src/commands/quest.commands.rs:45,96,163,203,235` and
`src-tauri/src/services/cli.service.rs:780,904,921,1172,1177`.

### The growth has no limit

No code deletes a row of `sync_outbox`, `sync_acks` or `sync_seen`. The only
delete on these tables is the 30-day removal of received tombstones
(`src-tauri/src/services/space-sync/apply.rs:278-290`). An acknowledged
change stays in the outbox. The growth does not depend on a peer that is
away: it is the same when all peers are online.

Measured sizes (the JSON of a typical one-line quest): the quest payload is
about 380 B, a `SyncEvent` frame is about 810 B, and an `Ack` frame is 64 B.
With the indexes, the estimates on disk are:

| Row | For each | Estimate |
|---|---|---|
| `sync_outbox` | edit, on the device that made it | 0.7 KB. A checklist quest adds its text, and one `checklist_activity` event for each item action |
| `sync_acks` | edit and peer | 0.2 KB |
| `sync_seen` | edit received | 0.1 KB |
| `sent_at` (memory) | peer and event, for the life of the process | 0.15 KB (`src-tauri/src/services/space-sync.service.rs:226,1037-1049`) |

At 30 edits a day with two peers, this is about 7.7 MB of outbox and 4.4 MB
of acks in a year, on each device.

### The query that finds the pending changes fails in time

`load_unacked_events_for_peer` reads all event ids that a peer acknowledged
and gives them to SQLite as `NOT IN (?, ?, …)`, one bound value for each id
(`outbox.repository.rs:118-143`). The bundled SQLite accepts at most 32 766
bound values. After about 32 700 acknowledged events from one peer, the
query is expected to fail with "too many SQL variables", and that peer gets
no more changes. At 30 edits a day, this is after about three years. This
comes from the code; it was not reproduced. Before it fails, the query is
O(history) on each tick for each peer. For a peer with no session, the tick
reads and walks all its pending events and sends none
(`space-sync.service.rs:1102-1132`, `:540-546`).

### The first sync of a space sends all history

`BootstrapStart` sends every event of the space in the outbox, oldest first
(`space-sync.service.rs:881-890`, `outbox.repository.rs:163-174`). This is
all history, not the state: about 11 000 events (about 9 MB) after one year,
where the rows of the space are about 300 KB. On Bluetooth, each event is two
fragments of 501 B (`docs/adr/0004-android-background-sync-daemon.md:127`),
plus one acknowledgement.

### Defects

1. **A delete has no LWW check.** The receiver deletes the row whatever its
   version (`apply.rs:364-367`). A received tombstone is never read
   (`sync_upsert_quest`, `quest.repository.rs:1165-1176`). Example: A deletes
   quest q at t2. B edits q at t3, after t2. On A, the edit wins over A's
   delete event and q comes back. On B, the delete removes q. The two devices
   stay different.
2. **The LWW compares the wrong thing.** An incoming change is compared with
   the receiver's own newest outbox event for the entity
   (`apply.rs:369-374`, `outbox.repository.rs:145-160`), not with the stored
   row. Changes that came from peers are not in the outbox. So an older
   change from a third device wins over a newer change from a second device.
3. **There is no relay.** A received change does not go into the outbox, and
   the bootstrap sends only the device's own events. With pairs A-B and B-C,
   C never gets the quests that A made.
4. **A backup import does not sync.** `backup.service.rs` writes rows and
   records no change, so the imported quests stay on one device.
5. **The row clock is in whole seconds.** The row `updated_at` uses
   `utc_now` (`src-tauri/src/services/db.service.rs:115`); only the event
   uses milliseconds (`:129-133`). A device whose clock is behind loses with
   an edit that it made after it received the change.

### Constraints

- Sync is whole-row last-writer-wins (LWW). The tie rule is in
  `SyncEventEnvelope::wins_over` (`src-tauri/src/services/channel.service.rs:716-733`).
- A checklist quest and a checklist day mark merge item by item before the
  LWW (#128; `apply.rs:339-354,392-449`,
  `quest.repository.rs:1226-1307`). `checklist_base` is local to each device
  and is not sent (`src-tauri/src/models/quest.rs:159-163`).
- Bluetooth carries iroh QUIC packets as GATT fragments of about 500 B. A
  weak link can give 12 B (`docs/adr/0004-android-background-sync-daemon.md:149`).
  A frame is a 4-byte length and JSON, at most 16 MiB
  (`channel.service.rs:516-519,654-661`).
- Each device has an Ed25519 key, the iroh secret key in the setting
  `device.secret_key` (`src-tauri/src/repositories/device.repository.rs:79-95`).
  Pairing pins the peer's public key in `paired_devices.endpoint_id`
  (`device.repository.rs:140-176`, called from
  `src-tauri/src/services/device.service.rs:1048`). The TLS handshake proves
  it (ADR-0009 D8; `find_by_key`, `device.repository.rs:205-213`).

## Decisions

### D1 — SpaceSync sends the difference of the state, not a log of changes

Two devices find the rows that are different between them and send only
those rows. Nothing is stored for each change, and nothing is stored for each
peer, except one mapping row (D9). The outbox, the acknowledgements, the seen
events and the resend state are removed (D11).

### D2 — Each synced row has a version, an origin and a signature

Each synced row has these sync columns:

| Column | What it is |
|---|---|
| `sync_version` | the hybrid logical clock (HLC) time of the write: `<physical ms, 13 digits>.<counter, 5 digits>` |
| `sync_origin` | the device id that wrote this version |
| `sync_signer` | the public key that signed this version (32 bytes) |
| `sync_signature` | the Ed25519 signature (64 bytes), or NULL while the row waits for its seal |

The LWW order is (`sync_version`, `sync_origin`, item id), compared as bytes.
The version text is fixed width, so a text comparison is the time order.

**The clock is an HLC, not the wall clock.** An HLC (Kulkarni et al.; also
CockroachDB and the Actual Budget sync) is the larger of the wall-clock time
and the newest version that the device has seen, with a counter for writes
in the same millisecond.

- Wall clock (what Fini does now): it is simple, and its order is the time of
  the edit. But a device with a slow clock loses with an edit that it made
  after it received the change (defect 5). Two writes in the same millisecond
  have the same version.
- HLC (the decision): a write always gets a version above every version that
  the device has seen. So an edit made after a received change wins over that
  change, whatever the clocks show. It still reads as a time, so "the newer
  edit wins" stays true for devices whose clocks agree.
- The receive rule: when a device applies a row, its clock goes to at least
  the row's version. At the end of each reconciliation, the two devices send
  their clocks, and each one goes to the larger (D5). A device with a clock in
  the future pulls the other clocks forward. The order stays correct; only the
  physical part reads wrong. Fini logs a warning when a received version is
  more than one hour ahead of the local clock (question 3).

**Where the version is written.** SQLite triggers on each synced table write
it, so no write path can leave it out. This includes the CLI, which is a
separate process, and the backup import:

- `AFTER INSERT` and `AFTER UPDATE OF <the synced columns>` set
  `sync_version`, `sync_origin` = this device, and `sync_signature = NULL`.
  The list of columns leaves out `checklist_base` and the sync columns, so an
  update of local data only makes no new version.
- One row, `sync_clock (physical_ms, counter)`, holds the HLC. The trigger
  computes `max(physical_ms, unixepoch('subsec') * 1000)`. The counter counts
  writes in the same millisecond.
- The triggers do nothing while the connection has a row in the temporary
  table `temp.sync_applying`. The apply step of SpaceSync adds that row, so a
  received row keeps its version, origin and signature. cr-sqlite uses the
  same method with an internal sync bit.

**The seal.** A trigger cannot sign, because the key is in the Rust process.
`SyncStateRepository::seal` finds the rows with `sync_signature IS NULL` (a
partial index) and signs each one (D7). SpaceSync seals at the start of each
tick, and the CLI seals before it exits. A row with no seal is not offered
to a peer.

### D3 — One digest tree for each space, by range-based set reconciliation

An **item** is one synced key in a space: (64-bit time, 32-byte id).

- The time is the physical part of `sync_version`. Items sort by time, so the
  recent changes are near each other, and the search splits fewer ranges.
- The id is `sha256` of the signed message of the row version (D7). It
  covers the space, the entity type, the key, the version, the origin and the
  content. It does not cover the signer or the signature, so two devices that
  hold the same version of a row have the same item.
- Each key in a space gives one item: the live row in that space, or its
  tombstone in that space (D6), whichever has the higher version.
- The entity types are `quest`, `quest_occurrence` (key
  `quest_id:period_key`, `src-tauri/src/models/quest_occurrence.rs:62-64`) and
  `checklist_activity`. A mark and an activity are in the space of their
  quest. All types are in one set for each space.

Two devices compare their sets with **negentropy**, the range-based set
reconciliation protocol of the Nostr relays, in its Rust crate (MIT). The
digest tree is not stored. Each level is computed in the run from the
sorted list of items:

```
range fingerprint = hash(sum of the item ids mod 2^256, item count)
  run 1: the whole set              one fingerprint
  run 2: a range that differs       16 sub-ranges, one fingerprint each
  run 3: a small range (< 32 items) the list of its item ids
  end:   each side knows which ids it has and which it needs
```

A fingerprint is additive, so the fingerprint of a sub-range costs no new
hash of its items. It does the same job as a Merkle tree (a hash of hashes),
with no tree to keep up to date. The cost is O(d · log n) bytes for d
different items in a set of n.

The item list of a space is built from SQL at the start of a run. For 10 000
rows, the estimate is about 20 ms on a phone (one `sha256` of each signed
message, then a sort). SpaceSync keeps the list in memory until a write
changes the space.

### D4 — The session flow (`fini/sync/7`)

The frames of a session:

| Frame | Fields | Replaces |
|---|---|---|
| `MappingState` | for each space of the pair: `enabled_at`, `end_of_sync_at` | `SpaceSyncEnd` and `peer_control_outbox` |
| `Signers` | the signers that this device trusts (D8) | — |
| `ReconcileOpen` | `run_id`, for each space: `space_id`, `item_count`, `msg` | `BootstrapStart` |
| `Reconcile` | `run_id`, `space_id`, `msg` | — |
| `Rows` | `run_id`, `space_id`, `rows: [SyncRow]` in batches of at most 32 KB | `SyncEvent` |
| `RowsWanted` | `run_id`, `space_id`, `ids` | — |
| `ReconcileDone` | `run_id`, `space_id`, `clock` | `BootstrapEnd`, `Ack` |

`msg` is a negentropy message in base64. A `SyncRow` is (`entity_type`,
`key`, `space_id`, `sync_version`, `sync_origin`, `sync_signer`,
`sync_signature`, `payload`). A NULL payload is a tombstone. The negentropy
frame limit is 4 KB on Bluetooth (about 9 fragments) and 64 KB on the
network.

```
session up
  ├─ both: MappingState, Signers
  ├─ the device with the smaller id: ReconcileOpen for each common mapped space
  │    (item_count = 0 on one side: that side gets all items, no search)
  ├─ Reconcile … Reconcile      until the initiator knows have / need
  ├─ initiator: Rows (have) + RowsWanted (need)
  ├─ responder: Rows (wanted)
  └─ both: ReconcileDone (clock); record last_reconciled_* (D9)
local write while a session runs
  ├─ seal, then push Rows for the changed keys at once   (live push)
  └─ a run for that space after 30 s with no writes      (the check)
held session with no writes
  └─ a 1-round-trip check of each space on each keeper tick
```

The live push keeps an edit as quick as today: one frame, no
acknowledgement. The keys to push stay in memory for the session only; they
are not a log. Correctness comes from the runs: a push that is lost shows
up as a different item in the next run. The check on each tick also finds
the writes of the CLI process, which sends no event to the app.

**Estimates.** The assumptions are: n = 2 000 items in the space (n = 10 000
for the last row); a row of about 0.9 KB as a frame (payload, version, origin,
key and signature); Bluetooth fragments of 501 B; Bluetooth 50–150 ms for
each round trip and 10–40 KB/s; network 1–5 ms for each round trip. The
Bluetooth figures are assumptions, to measure in phase 5.

| Rows that differ | Round trips | Bytes, search | Bytes, rows | Bluetooth fragments | Bluetooth time | Network time |
|---|---|---|---|---|---|---|
| 0 | 1 | 2 × ~110 B (all spaces in one frame) | 0 | 2 | < 0.2 s | < 10 ms |
| 1 | 3–4 (live push: 0.5, one way) | ~1.5 KB | 0.9–1.8 KB | 6–8 (push: 2) | ~0.5 s | < 20 ms |
| 100, recent (offline edits) | 3–4 | ~12 KB | 90–180 KB | 200–390 | 3–20 s | < 0.5 s |
| 100, spread in time | 6–9 (4 KB frames) | ~35 KB | 90–180 KB | 250–440 | 4–25 s | < 0.5 s |
| 10 000, one side empty (first sync) | 1, then rows | ~0.2 KB | ~9 MB | ~18 000 | 4–15 min | 2–10 s |
| 10 000, both full, all different | ~80 (BLE), ~6 (network) | ~640 KB | ~18 MB | ~37 000 | 8–30 min | 5–20 s |

For comparison, today one change is one `SyncEvent` (2 fragments) and one
`Ack`, and the first sync of a space sends all history. The last row of the
table is a worst case that the migration prevents (D12: rows that are the
same on two devices get the same item).

### D5 — Relay, the apply rules, and the merges

**Relay.** The set of a space contains all rows of the space, the rows that
came from peers too. With pairs A-B and B-C, B's set has A's rows, so C gets
them. Each row keeps its origin, its signer and its signature from A (D8).
No relay code is necessary.

**Apply rules.** SpaceSync applies the rows of a run in one write
transaction with `temp.sync_applying`. A parent comes before its children
(space, quest, then the others), as today (`space-sync.service.rs:1162-1175`).
A child with no parent waits for a later run. For one key, "the version here"
is the highest version of the live row (in any space) and of its tombstones.

- A live row wins if its version is higher than the version here. Then
  SpaceSync writes it, and removes the tombstones of the key that have a lower
  version.
- A tombstone for space S deletes the live row only if the row is in S and
  its version is lower. Then SpaceSync stores the tombstone. If not, it stores
  the tombstone only when the tombstone has a higher version than the one in S
  here.
- A row that loses changes nothing here. The peer gets the newer version in
  the same run, because the two items are different.
- After each applied row, the local clock goes to at least the row's version
  (D2).

**Checklist quests and checklist day marks (#128, ADR-0012 D5).** The merge
code stays (`sync_merge_quest_checklist`, `sync_merge_occurrence_checklist`).
An incoming checklist row merges its description with the local row,
whichever version wins. When the merge result is not the same as the
incoming row, SpaceSync writes the result as a local write, outside the
guard, so it gets a new version from this device. The next run sends it back.
This replaces the convergence events (`apply.rs:301-337`). The merge is
idempotent: when the two sides hold the same merged text, the merge makes no
new version. `checklist_base` stays local to each device.

**Occurrence marks (ADR-0012 D11).** A mark is its own row and its own item,
keyed by (`quest_id`, `period_key`). A removed mark is a tombstone. A mark
moves with its quest, because its space is the quest's space.

**Checklist activity.** An activity row is never changed after it is
written. Its version is its first version, and it never has a conflict.

**Rank keys.** `order_rank` is a column of the quest row, under the LWW of
the whole row. A move writes one quest. Two moves of different quests on two
devices both stay, because a rank key move changes no other key. A rebalance
gives N quests new versions, so it sends N rows. Two rebalances at the same
time on two devices can mix the keys of the two spreads. The equal-neighbour
check of `move_quest` corrects this at the next move. This is a known limit
of whole-row LWW.

**Backup import.** An imported row is an ordinary local write: the triggers
give it a version, the seal signs it, and the next run sends it. The conflict
choices of the import (`backup.service.rs`) stay as they are. An imported
version is newer than the versions on the peers, so it wins, as any edit
does. This removes defect 4.

### D6 — A delete is a versioned tombstone under the same LWW

A tombstone is one row of `sync_tombstones (entity_type, entity_key,
space_id, sync_version, sync_origin, sync_signer, sync_signature,
recorded_hlc, PRIMARY KEY (entity_type, entity_key, space_id))`.

- An `AFTER DELETE` trigger writes it for a local delete. Its version is the
  next HLC time.
- An `AFTER UPDATE OF space_id` trigger on `quests` writes a tombstone in the
  old space, with the version of the moved row. A peer that syncs only the old
  space then deletes the quest. A peer that syncs both spaces deletes it in the
  old space and has it in the new one.
- An insert of a key with a tombstone removes the tombstone, because the new
  row has a higher version.
- `recorded_hlc` is the local clock when the tombstone was stored here (for
  D9).
- A tombstone is signed as a row is (D7).

The tombstone is compared by version as a row is (D5), so a delete and a
newer edit end the same way on all devices: the newer one wins. This removes
defect 1.

### D7 — Each row version is signed by the device that wrote it

**What is signed.** The signed message of one row version is:

```
"fini/sync-row/1" 0x00          domain tag
lp(space_id)
lp(entity_type)
lp(entity_key)
lp(sync_version)
lp(sync_origin)
u8 kind                          0 = live row, 1 = tombstone
sha256(canonical payload)        32 zero bytes for a tombstone
```

`lp(x)` is the length of x as a 4-byte big-endian number, then the UTF-8
bytes of x. The canonical payload is the RFC 8785 (JSON Canonicalization
Scheme) form of the synced columns of the row. It does not contain the sync
columns or the columns that are local to a device (`checklist_base`). The
item id is `sha256` of the signed message (D3), and the signature is
`Ed25519(secret key, signed message)`.

The space is in the message, so a relay cannot move a row to another space.
The version and the origin are in the message, so a relay cannot make an old
row win.

**The key.** Fini uses the iroh key of the device (`iroh::SecretKey::sign`,
`iroh::PublicKey::verify`). Pairing already pins it and the TLS handshake
already proves it, so no new key and no new exchange are necessary. The
domain tag separates the two uses: TLS 1.3 signs a context string that
starts with 64 spaces, so a sync signature can never be a valid TLS signature
and the reverse. A separate key would add a second pin to pairing for no
gain.

**Verification on receive.** For each received row:

1. The signer is in the signers of this session (D8), and it is not revoked.
2. `sync_signer` is a key of `sync_origin`, except for a `legacy` row (D12),
   which a trusted signer seals.
3. The Ed25519 signature is valid for the signed message that this device
   computes again from the row.
4. In a `Rows` reply to `RowsWanted`, the item id is one that was asked for.

A row that fails a check is rejected. It is logged, and the count shows on
the Device page. A row with no signature is rejected: `fini/sync/7` has no
unsigned rows.

**The cost** (estimates, to measure in phase 2):

| | For each row version | 10 000 rows |
|---|---|---|
| Storage (version, origin, signer, signature) | ~190 B | ~1.9 MB |
| Sign on a mid-range phone | 20–60 µs | 0.2–0.6 s (once, at the migration) |
| Verify on a mid-range phone | 50–150 µs | 0.5–1.5 s (first sync of a full space) |

A live edit signs one row and verifies one row: less than 1 ms.

### D8 — Trust comes from pairing, and a relayed row keeps its signer

The table `sync_signers (public_key PRIMARY KEY, device_id, source,
introduced_by, added_at, revoked_at, retired_at)` holds the keys that this
device accepts.

- `source = self`: this device's key.
- `source = paired`: a key pinned at pairing (`DeviceRepository::save`). A new
  pairing of a reinstalled device pins a new key. The old key gets
  `retired_at`: its stored rows stay valid, and a new version signed by it is
  rejected.
- `source = introduced`: a key that a paired device sent in `Signers`. The TLS
  handshake proves who sent the frame, so the frame needs no signature of its
  own. With pairs A-B and B-C, B introduces A to C, and C accepts the rows that
  A signed and that B relays. An introduced key can be introduced again, so
  trust follows the paired devices of one person.

**The signers of a session.** A run uses only the signers that both devices
trust. The two `Signers` frames give the common set. A row whose signer is
not in the common set is not an item of that run, on either side. So a row
that one side cannot accept is not offered again and again.

**Revocation at unpair.** `DeviceService::unpair`
(`device.service.rs:1100-1102`) sets `revoked_at` on the keys of the removed
device. Then:

- A new version signed by a revoked key is rejected.
- This device signs again, as its own versions, the stored rows whose signer
  is revoked (it adopts them). Their content does not change. So the data
  stays, which agrees with the unpair dialog ("nothing is deleted",
  `specs/e2e/multi-device/tests/device-page.spec.ts:149`), and it keeps
  syncing to the other devices. The adoption costs one signature and one row
  transfer for each adopted row, one time.
- The next `Signers` frame carries the revocation to the paired devices. Each
  of them applies it and shows a notice. It does not unpair the device there
  (question 6).

### D9 — Tombstone GC, the TTL, and the rejoin rule

`pair_space_mappings` gets `last_reconciled_at` (the wall clock) and
`last_reconciled_hlc` (the local HLC). SpaceSync writes them at the end of
each run whose sets became equal. `SpaceMappingRepository::start` no longer
clears them (`src-tauri/src/repositories/space-mapping.repository.rs:126-130`),
because the rejoin rule needs them. `sync_space_state (space_id,
gc_horizon_hlc)` holds the newest `recorded_hlc` of a tombstone that GC
removed.

**GC rule.** A tombstone of space S goes when every active mapping of S has
`last_reconciled_hlc > recorded_hlc`. The tombstone was in this device's set
during that run, so the peer has it or a newer version. An unpaired peer and
an ended mapping do not count. This rule is exact.

**TTL.** A mapping with `last_reconciled_at` older than 90 days does not
stop the GC. A peer that is away forever then costs one mapping row and
nothing else.

**Rejoin rule.** A peer P is in rejoin mode for S when `last_reconciled_hlc`
of P is lower than `gc_horizon_hlc` of S: tombstones that P did not get are
gone. In rejoin mode, a row that P offers, that is not here, and that has no
tombstone here is a delete that P did not get when all of these are true:

- its version is not above `last_reconciled_hlc` of P (L);
- its origin is P or this device.

Then this device sends a tombstone, and does not insert the row. This is
exact: at the end of the run at L, both clocks went above L (D4,
`ReconcileDone`), so a row of P or of this device with a version at or below
L was in both sets then. Its absence here now is a delete.

All other rows follow the LWW. The rule only lets a row come back; it never
deletes data that it cannot prove is deleted. There are two edge cases:

- P edited a row after L, and this device deleted the row before that edit
  and removed the tombstone. The row comes back.
- A row from a third device has a version at or below L (that device's clock
  was behind), and it was deleted here and the tombstone was removed. The row
  comes back.

Both need more than 90 days away. A first sync (L is NULL) has no rejoin
mode: the two sets join.

**The bound.** The tombstones are the deletes and moves since the slowest
active peer's last run, and never older than the TTL. Nothing grows with each
edit or with each peer.

### D10 — The mapping state replaces the kept frames

At the start of each session, the two devices send `MappingState`. For each
space, the newer of (`enabled_at`, `end_of_sync_at`) wins. An end that is
newer than the local start ends the sync here, as `SpaceSyncEnd` does today.
A newer start does not start a sync here: a start still needs the request
(`SpaceMappingUpdate`) and the approval of the person. "Syncing the space
again removes the end that was not sent" (ADR-0008 D14) follows from the
newer `enabled_at`. The end is in the mapping row, so a restart does not lose
it. So `peer_control_outbox` is not necessary.

### D11 — What is removed

| Removed | Where |
|---|---|
| tables `sync_outbox`, `sync_acks`, `sync_seen`, `tombstones`, `peer_control_outbox` | migration 32 |
| `OutboxRepository` and its file | `src-tauri/src/repositories/outbox.repository.rs`, `repositories/mod.rs` |
| the outbox models | `src-tauri/src/models/sync.rs:6-84` |
| `SyncEventEnvelope`, `wins_over`; the frames `SyncEvent`, `Ack`, `BootstrapStart`, `BootstrapEnd`, `SpaceSyncEnd` | `channel.service.rs:702-733,807-833` |
| `sent_at`, `EVENT_RESEND_COOLDOWN`, `MAX_EVENTS_PER_PEER_PER_TICK`, `BOOTSTRAP_RETRY`, `outbox_kept`, `send_kept_frames`, and the maps of incoming events, acknowledgements and ends | `space-sync.service.rs:81-91,203-208,226,236-238,820-829` |
| every `OutboxRepository::record_*` and `emit_sync_event` call; `emit_missing_sync_events` | `quest.commands.rs`, `cli.service.rs`, `quest.repository.rs:1186-1220`, `apply.rs:301-337` |

**The Repository trait.** The trait does not change (`fn new(conn)`,
`repositories/mod.rs`). The list of repositories that implement it changes:

```
Repository<'a>
  ├─ ChannelRepository        channels
  ├─ DeviceRepository         paired_devices
  ├─ QuestRepository          quests, quest_occurrences, focus_history, checklist_activity
  ├─ ReminderRepository       reminders
  ├─ SignerRepository         sync_signers                                   (new)
  ├─ SpaceMappingRepository   pair_space_mappings
  ├─ SpaceRepository          spaces
  └─ SyncStateRepository      the sync columns, sync_tombstones, sync_clock,  (new)
                              sync_space_state: items, rows by id, seal, GC
```

`OutboxRepository` goes. `npm run lint:rust` checks that each new repository
has its `impl Repository`. [`db.md`](../db.md) changes with it: the tree, and
rule 6 becomes "a trigger writes the version of a synced row in the same
transaction as the change".

### D12 — The migration of the data

Migration 31 adds the sync columns, `sync_clock`, `sync_signers`,
`sync_tombstones`, `sync_space_state` and the triggers. Then it fills them:

- `sync_version` of each row is its `updated_at` in milliseconds with counter
  0, and `sync_origin` is `legacy`. A `checklist_activity` row uses
  `created_at`. A row that is the same on two devices has the same
  `updated_at` on both, because the apply step writes the peer's
  `updated_at`. So it gets the same item on both devices, and the first run
  sends only the rows that really differ.
- `sync_tombstones` gets the outbox `delete` events whose entity is not here
  any more, and the received `tombstones`, with origin `legacy`.
- `sync_signers` gets this device (`self`) and each `paired_devices.endpoint_id`
  (`paired`).
- `last_reconciled_*` stays NULL. So the first run has no rejoin mode.
- At the first start, the seal signs all rows (D7).

The old tables stay for one release, so a rollback to the earlier app is
possible. Migration 32 drops them (question 8).

### D13 — The protocol version and devices on two versions

`SYNC_ALPN` becomes `fini/sync/7` (`channel.service.rs:490-496`). Devices on
`fini/sync/6` and `fini/sync/7` share no ALPN, and they do not connect. This
is the rule of `PeerFrame` (`channel.service.rs:801-804`). No change is lost:
the state is in the rows, and migration 31 keeps the deletes as tombstones.
When both devices have the new version, the first run sends the
differences.

**A notice for the person.** Today a version difference shows only as a
connect that fails. With this decision:

- A device on version 7 also accepts the older sync ALPNs (`fini/sync/6`) on
  its endpoint (`channel.service.rs:240`). It closes such a connection at once
  with a new close code 5 (`CLOSE_OTHER_VERSION`), and records that the peer
  runs an older Fini.
- A connect from version 7 to an older peer fails in the TLS handshake with
  the alert `no_application_protocol`. The device records that the peer runs
  another Fini.
- The Device page shows on the channel row: "<peer> runs another version of
  Fini. Update the two devices to sync." The status comes from
  `SpaceSyncService::status`.

## Alternatives considered

| Alternative | Why it is not chosen |
|---|---|
| **Keep the outbox, with pruning**: remove a change that all peers acknowledged, replace an older change of the same entity, remove the queue of a peer that is away for N days, then a full-state sync | Less work, and the growth stops. But the acknowledgements, the seen events, the resends and the 64-event limit stay, and so do defects 1–5. The full-state sync that it needs for a returning peer is half of this decision. |
| **Sequence numbers for each device** (cr-sqlite `db_version`, CouchDB `_changes?since=`): send the rows with a number above the peer's watermark | One round trip, and the fewest bytes. But it is correct only while each watermark is correct. A reinstall, a restored database or a cloned device makes a silent difference that nothing finds. To find it, it needs a state comparison, which is this decision. A watermark path can come later on top of this decision, if the measured round trips need it. |
| **cr-sqlite** (CRR tables, LWW for each column) | A loadable extension. Fini would build it for Android, Linux, Windows and Flatpak and load it under Diesel. It rewrites the tables into its own shadow tables, which conflicts with Fini's migrations. The project has had almost no commits since 2024. |
| **Automerge, Yjs / Yrs** | Document CRDTs that keep an operation history, which grows as the outbox does. Fini's data is rows that Diesel reads with SQL, not documents. |
| **SQLite session extension** (changesets) | Needs SQLite built with `SQLITE_ENABLE_SESSION`, and Diesel has no interface for it. It still needs a log or a cursor for each peer. |
| **A blockchain or another append-only signed log** | A consensus between parties that do not trust each other is not necessary: all devices belong to one person and are paired. An append-only log cannot be pruned, which is the problem that this decision removes. Signatures (D7) give the authenticity without a chain. |
| **A Merkle tree stored in the database** | It is quicker to read, but each write must update it in the same transaction. At Fini's sizes (10⁴ rows), the tree that negentropy computes in the run is quick enough (D3). |

## Consequences

- The sync data is the live rows, about 190 B more for each row, and the
  tombstones of D9. Nothing grows with each edit or each peer. A peer that is
  away forever costs one mapping row.
- Deletes, moves, relay and backup import behave the same on all devices
  (defects 1–5 go).
- A row that a device did not sign cannot come from another device. A relay
  cannot change a row, move it to another space, or make an old version win.
- One edit costs about what it costs now (the live push). An idle check costs
  one small round trip for each session. The first sync sends the state, not
  the history.
- New dependencies: the negentropy crate, `sha2`, and an RFC 8785 encoder
  (`Cargo.toml`).
- The Device page queue ("changes that wait") counts the local rows with a
  version above the last run with that peer, not outbox events.
- A device on version 6 and a device on version 7 do not sync, and the
  person sees why.

### Phases and files

| Phase | Work | Files |
|---|---|---|
| 1. Versions and tombstones (no wire change) | migration 31; triggers; `sync_clock`; the `temp.sync_applying` setup on each connection; the seal | new `src-tauri/migrations/00000000000031_sync_state/{up,down}.sql`; new `src-tauri/src/repositories/sync-state.repository.rs`, `signer.repository.rs`; changed `src-tauri/src/schema.rs`, `repositories/mod.rs`, `models/quest.rs`, `models/quest_occurrence.rs`, `models/checklist_activity.rs`, `services/db.service.rs` |
| 2. Reconciliation core (no input or output) | items, negentropy steps, the signed message, apply rules, merges | new `src-tauri/src/services/space-sync/reconcile.rs`, `space-sync/signing.rs`; `models/sync.rs` rewritten (`SyncRow`, `SyncVersion`, `SyncTombstone`, `Signer`); `space-sync/apply.rs` rewritten; `Cargo.toml` |
| 3. Wire and session | `fini/sync/7`; the new frames; the run driver; live push; `MappingState`; status and queue | `services/channel.service.rs`, `services/space-sync.service.rs`, `commands/space-sync.commands.rs`, `commands/quest.commands.rs`, `services/cli.service.rs`, `repositories/quest.repository.rs`, `repositories/space-mapping.repository.rs`; frontend `src/stores/space-sync.store.ts`, `src/stores/device.store.ts`, `src/components/settings/device/SyncQueueSection.vue`, `src/views/settings/DeviceView.vue` and their specs |
| 4. Signers, GC and rejoin | `Signers` frame; revocation and adoption at unpair; GC; TTL; rejoin mode; version notice | `services/device.service.rs`, `space-sync/reconcile.rs`, `repositories/signer.repository.rs`, `channel.service.rs` (close code 5) |
| 5. Removal and docs | migration 32; delete `outbox.repository.rs`; update `docs/db.md`, `docs/architecture/communication.md`, `docs/architecture/code-design.md`, `specs/backup/README.md` | as listed |

### Tests

- **Two-database convergence property test** (phase 2): two in-memory
  databases A and B, a random sequence of create, edit, delete, move, tick,
  rank move and rebalance on each one, then runs over a mock link until the
  sets are equal. Then the rows are equal on A and B, and a second run sends
  nothing. Variants: three databases in a line (relay); a clock in the
  future; a peer that is away past the TTL (rejoin); a signer revoked in the
  middle.
- Unit tests: the triggers (a local write makes a version; a guarded write
  does not; a write of `checklist_base` only does not; a delete and a move
  make tombstones); the HLC (same millisecond, clock goes back, receive
  rule); the signed message (fixed vectors; a changed byte fails); each
  verification check; the GC rule; the rejoin rule; the migration
  (`services/db/migrations-tests.rs`: the legacy rows have the same items on
  two databases).
- Ported tests: the checklist and mark merge tests of `apply.rs`, and the
  session tests of `space-sync.service.rs` (the `Fixture`).
- Multi-device e2e (`specs/e2e/multi-device/tests/`): update
  `personal-space-sync.spec.ts`, `zz-personal-space-live-quest-sync.spec.ts`,
  `peer-sync-over-ble.spec.ts`, `device-page.spec.ts` (the queue) and
  `specs/e2e/feature-plane-cli.spec.ts` (it reads `outbox_event_count`). New
  specs: offline edits on both sides; a delete while the peer is offline; an
  edit after a delete; a backup import that syncs; a peer that is silent
  for a long time adds no rows; a version-6 peer shows the notice.
- Hardware: the desktop and phone pair over Bluetooth, timed through the
  devtools bridge, for the rows 0, 1 and 100 of the D4 table.

## Unresolved questions

1. The TTL for silent peers: 90 days? Or no TTL, so that tombstones stay until
   each active peer has a run (exact, with no edge case), and the Device page
   offers to unpair a device that is away for a long time?
2. Are the two edge cases of D9 acceptable (a deleted row can come back after
   more than 90 days away)?
3. A received version far in the future: only a warning (D2), or a limit
   (for example one day) above which the row is rejected?
4. Should `focus_history` and the space names sync? They have apply code today
   but nothing sends them.
5. Should introduced signers (D8) be accepted automatically, or should the
   person confirm each new device that a paired device introduces?
6. Revocation from another device: apply it and show a notice (D8), or also
   unpair the revoked device on each device?
7. Should a backup import sync at once (D5), or stay local until the person
   edits the imported quests?
8. Keep the old tables for one release (migration 32 later), or drop them in
   migration 31?
9. The live push (D4): keep it, or use only runs (simpler; an edit then takes
   3–4 round trips)?
