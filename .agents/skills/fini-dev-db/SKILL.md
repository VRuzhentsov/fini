---
name: fini-dev-db
description: Guide safe, low-debt SQLite and Diesel schema migrations in Fini.
---

# Fini Database Development

Use this skill for Fini SQLite schema changes, Diesel schema updates, data migrations, migration rollback decisions, and database migration tests.

## Database Design Preference

Fini migrations should leave the database in one clear current state. Do not create permanent compatibility tables, backup tables, shadow copies, or `_old`/`_backup` tables as part of a normal migration. They add schema debt, multiply maintenance paths, and conceal incomplete migrations.

When SQLite requires a table rebuild to change a constraint or column type, use a **transient replacement table** only within the migration transaction/script:

1. Create a clearly temporary `<table>_replacement` table with the complete target schema.
2. Copy rows with explicit transformations for every changed field.
3. Drop the original table.
4. Rename the replacement to the canonical table name.
5. Recreate required indexes and re-enable foreign keys.

The replacement table must not remain after a successful migration. Do not name it `_old`, `_backup`, or present it as retained recovery data.

## Migration Contract

1. Define the final schema first: types, nullability, defaults, constraints, indexes, and foreign keys.
2. Map legacy values explicitly in SQL. Unknown or malformed legacy values must follow a documented safe fallback.
3. Preserve every unaffected column and relationship during a rebuild.
4. Decide reversibility deliberately. If a down migration is supported, it must recreate the actual prior schema and map values back explicitly; it must not merely execute successfully with the new schema still present.
5. Do not add backup/compatibility tables to avoid deciding the migration contract. If lossless reversal is impossible, document and approve an irreversible migration instead.

## Verification

Before commit:

1. Test a real prior-version database upgraded through the migration; assert schema and migrated rows.
2. If a down migration exists, test upgrade then downgrade; assert the prior schema and mapped values.
3. Assert no transient replacement table remains, using `sqlite_master` or equivalent schema inspection.
4. Update Diesel schema/model code consistently with the final schema.
5. Run the focused Rust migration test, scoped formatting, and `git diff --check`.

Use `SimpleConnection::batch_execute` for complete migration SQL scripts in tests. Do not split SQLite migration scripts on semicolons.

## Boundaries

- Database migrations change persisted shape and data only; product lifecycle rules belong in services.
- Diesel persistence for critical models belongs in the repository layer.
- Do not create durable database objects solely for migration convenience.
