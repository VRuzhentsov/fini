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
   conn.immediate_transaction(|conn| {
       QuestRepository::new(conn).update(&id, patch)?;
       OutboxRepository::new(conn).record_change(&device_id, &change)
   })
   ```

5. A transaction that writes begins with `immediate_transaction`, not
   `transaction`. A deferred transaction that reads first and writes later
   fails at once when another connection wrote in between (SQLite
   `SQLITE_BUSY_SNAPSHOT`); `busy_timeout` does not help it. An immediate
   transaction takes the write lock at its start, so another writer waits
   for it. The tests in `db.rs` (`write_lock_tests`) show both.
6. A change to synced data writes its sync outbox rows in the same
   transaction as the change (transactional outbox). Then a change is never
   saved without its outbox rows, and the reverse.
7. Tell other parts about a change after the commit, on the event bus
   (`EventBusService`, ADR-0010 D8). The repository does not call them.
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

## Not done yet

- `AppDbConnection` (one `Mutex<SqliteConnection>`) and the services that
  open a new connection on each call (`with_conn` + `open_db_at_path` in
  `DeviceService`, `SpaceSyncService` and the channels) move to the pool.
- Quest writes and their outbox rows are two steps in the command; they
  become one transaction.
