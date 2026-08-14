---
name: fini-dev-db
description: Guide safe, low-debt SQLite and Diesel schema migrations in Fini.
---

# Fini Database Development

Use this skill for Fini SQLite schema changes, Diesel schema updates, data migrations, migration rollback decisions, and database migration tests.

## Database Design Preference

Fini is in open alpha. Treat the current schema as the only supported schema: do not add legacy mappings, conversion paths, compatibility tables, backup normalization, or rollback compatibility unless the user explicitly asks for them.

Use direct SQLite schema mutations (`ALTER TABLE ... ADD COLUMN`, `DROP COLUMN`, or `RENAME COLUMN`) whenever SQLite supports the required change. Do not use replacement tables for normal Fini schema work. If a required change cannot be made with direct SQLite mutations, stop and obtain an explicit user decision rather than silently introducing a rebuild or compatibility layer.

## Migration Contract

1. Define the final schema first: types, nullability, defaults, constraints, indexes, and foreign keys.
2. Implement the new contract directly; do not map old values. Use defaults for fields that are newly introduced or intentionally reset.
3. Preserve unaffected columns and relationships.
4. Decide reversibility deliberately. A migration may be explicitly irreversible; do not recreate a legacy contract merely to provide rollback.

## Verification

Before commit:

1. Test a real prior-version database upgraded through the migration; assert the final schema and defaults.
2. Confirm migrations use direct SQLite mutations only and leave no replacement, backup, shadow, `_old`, or `_backup` tables.
3. Update Diesel schema/model code consistently with the final schema.
4. Run the focused Rust migration test, scoped formatting, and `git diff --check`.

Use `SimpleConnection::batch_execute` for complete migration SQL scripts in tests. Do not split SQLite migration scripts on semicolons.

## Boundaries

- Database migrations change persisted shape and data only; product lifecycle rules belong in services.
- Diesel persistence for critical models belongs in the repository layer.
- Do not create durable database objects solely for migration convenience.
