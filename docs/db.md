# Database access

How Fini's code reaches the SQLite database. The code follows this page; when
they differ, the code is wrong.

## The parts

```
app start (lib.rs setup) or CLI start (run_cli)
  └─ DbPool                one r2d2 pool of SQLite connections, app.manage()
       │ pool.get()        a command or a service takes one connection
       ▼
     connection            used by one thread at a time (Rust &mut)
       │ XRepository::new(&mut conn)
       ▼
     repositories          the SQL of one kind of data, on that connection
```

- **Pool** (`DbPool`, `services/db.rs`): the connections of the process,
  ten at most (Diesel's `r2d2`). It is made once, at start, and Tauri keeps it
  (`app.manage(pool)`). The CLI makes its own pool when it starts. A test
  makes a pool of one connection.
- **Connection**: one open handle to `fini.db`. A command, a service call or
  a background task takes one with `pool.get()` for the time of its work, and
  the pool takes it back at the end. This is as Laravel gives one connection
  to each request.
- **Repository** (`repositories/*.repository.rs`): the only place that has
  the SQL for one kind of data (ADR-0010 D11). It is made on a connection,
  `QuestRepository::new(&mut conn)`, and it cannot exist without one. To make
  one costs nothing: it holds only the connection. Code that needs five
  repositories makes five on the same connection.

## The Repository trait

Each repository implements the trait `Repository` (`repositories/mod.rs`).
The trait holds what all repositories do in the same way: `new` makes the
repository on a connection. To call `new`, a caller has the trait in scope
(`use crate::repositories::Repository`).

```
Repository<'a>                trait: fn new(conn: &'a mut SqliteConnection) -> Self
  ├─ ChannelRepository        channels
  ├─ DeviceRepository         paired_devices
  ├─ OutboxRepository         sync_outbox, sync_acks, sync_seen, peer_control_outbox
  ├─ QuestRepository          quests, quest_occurrences, focus_history, checklist_activity
  ├─ ReminderRepository       reminders
  ├─ SpaceMappingRepository   pair_space_mappings
  └─ SpaceRepository          spaces
```

- A repository is one struct that holds `conn`. Its file has its
  `impl Repository` block and one inherent `impl` block
  ([`code-style.md`](code-style.md), "Write one inherent `impl` block for
  each type").
- The trait has no methods that read, write or delete rows. The methods
  with the same names (`get`, `create`, `update`, `delete`) do not have the
  same arguments and results in all repositories, so one generic method
  does not fit them. Add a method to the trait only when all repositories
  do it in the same way, and write it one time, as a default method.
- `npm run lint:rust-impl` fails when a `struct …Repository` has no
  `impl Repository for` in its file.

## Rules

1. Take a connection from the pool. Do not open the database file anywhere
   else (`SqliteConnection::establish`, `open_db_at_path`). Only `db.rs`
   opens it.
2. Take one connection for one piece of work, and give it back at the end.
   Do not keep a connection in a struct field, and do not hold one across an
   `.await`.
3. Do the SQL of a kind of data in its repository. A command or a service
   calls repository methods; it does not write SQL.
4. Writes that must be saved together go in one transaction on one
   connection. Every repository in the transaction is made on that
   connection:

   ```rust
   conn.write_transaction(|conn| {
       let change = QuestRepository::new(conn).update(&id, patch, scope, period_key, Some(&device_id))?;
       OutboxRepository::new(conn).record_change(&device_id, &previous_space_id, &change)
   })
   ```

5. A transaction that writes begins with `IMMEDIATE`, not as a plain
   `transaction`. `WriteTransaction::write_transaction` (`db.rs`) does this
   for code with `String` errors, and gives the closure's error back. A deferred transaction that reads first and writes later
   fails at once when another connection wrote in between (SQLite
   `SQLITE_BUSY_SNAPSHOT`); `busy_timeout` does not help it. An immediate
   transaction takes the write lock at its start, so another writer waits
   for it. The tests in `db.rs` (`write_lock_tests`) show both.
6. A change to synced data writes its sync outbox rows in the same
   transaction as the change (transactional outbox). Then a change is never
   saved without its outbox rows, and the reverse.
7. Tell other parts about a change after the commit, on the event bus
   (`EventBus`, ADR-0010 D8). The repository does not call them.
   Example: a quest command publishes `Event::QuestChanged`;
   `ReminderService` moves the reminder; `SpaceSyncService` wakes.

## Each connection is set up the same way

The pool sets these on each connection that it opens, in this order:

1. `PRAGMA busy_timeout = 15000`: a connection that finds the database
   locked waits up to 15 s. It comes first: the next step needs it.
2. `PRAGMA foreign_keys = ON`.
3. `PRAGMA journal_mode = WAL`: readers and one writer work at the same time.

Each pragma is its own statement. Diesel runs only the first statement of a
string with `;` in it.

The migrations run once, before the pool is shared: `db.rs` refuses a
database that has a migration this build does not know (a newer Fini wrote
it).

## Why a pool, not one shared connection

- Diesel's documentation shows a pool for code that runs on more than one
  thread, and Tauri's own SQL plugin keeps a pool in managed state.
- Fini works on several threads at the same time: UI commands, SpaceSync,
  the network and Bluetooth channels. With WAL they read in parallel; SQLite
  lets one of them write at a time, and the others wait (`busy_timeout`).
- A transaction needs all its writes on one connection. In Rust that is the
  connection that the transaction gives to its closure (`|conn|`), so no lock
  of our own is necessary.

One connection for the whole app (as Laravel has in one PHP request) was
considered. PHP runs one request at a time in its process; Fini does not.
One shared connection needs a lock, and a transaction across repositories
needs that lock to be re-entrant (`parking_lot::ReentrantMutex`). That was
rejected: more code of our own, and every read waits for every write.

## Schema migrations

Fini is in open alpha ([`development.md`](development.md), "No legacy in
alpha"). Treat the current schema as the only supported schema: do not add
legacy mappings, conversion paths, compatibility tables, backup
normalization, or rollback compatibility unless the user explicitly asks for
them.

Use direct SQLite schema mutations (`ALTER TABLE ... ADD COLUMN`, `DROP COLUMN`, or `RENAME COLUMN`) whenever SQLite supports the required change. Do not use replacement tables for normal Fini schema work. If a required change cannot be made with direct SQLite mutations, it needs an explicit user decision; do not silently introduce a rebuild or compatibility layer.

### Migration contract

1. Define the final schema first: types, nullability, defaults, constraints, indexes, and foreign keys.
2. Implement the new contract directly; do not map old values. Use defaults for fields that are newly introduced or intentionally reset.
3. Preserve unaffected columns and relationships.
4. Decide reversibility deliberately. A migration may be explicitly irreversible; do not recreate a legacy contract merely to provide rollback.

### Boundaries

- Database migrations change persisted shape and data only; product lifecycle rules belong in services.
- Diesel persistence for critical models belongs in the repository layer.
- Do not create durable database objects solely for migration convenience.

### Migration tests

- A migration test upgrades a real prior-version database through the migration and asserts the final schema and defaults.
- Migrations use direct SQLite mutations only and leave no replacement, backup, shadow, `_old`, or `_backup` tables.
- Diesel schema/model code agrees with the final schema.
- Use `SimpleConnection::batch_execute` for complete migration SQL scripts in tests. Do not split SQLite migration scripts on semicolons.
