---
name: fini-dev-db
description: Guide safe, low-debt SQLite and Diesel schema migrations in Fini. Use for schema changes, Diesel schema updates, data migrations, migration rollback decisions, and database migration tests.
---

# Fini database development

Use this skill for Fini SQLite schema changes, Diesel schema updates, data
migrations, migration rollback decisions, and database migration tests. It adds
to `dev`; follow that process first.

The rules are in `docs/db.md`: "Rules" for database access, and "Schema
migrations" for the migration contract, the boundaries and the tests.

1. Read `docs/db.md`, "Schema migrations".
2. Define the final schema first, then implement it with direct SQLite
   mutations. If a required change cannot be made with direct SQLite
   mutations, stop and obtain an explicit user decision.
3. Update Diesel schema/model code consistently with the final schema.
4. Before commit, run scoped formatting and `git diff --check`, and confirm
   that the migration leaves no replacement, backup, shadow, `_old`, or
   `_backup` tables.
5. In the stabilize phase (`fini-test`, "Stabilize"), test a real
   prior-version database upgraded through the migration and run the focused
   Rust migration test (`docs/db.md`, "Migration tests").
