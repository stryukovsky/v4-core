# Backend instructions

## PostgreSQL workflow

When a new database-backed struct or table is requested or required, include its
SQL scripts in the same change. Do not wait for a separate request for
migrations or cleanup scripts.

Store PostgreSQL SQL files under `backend/sql/`:

| Directory | Purpose                                                           | Example                  |
| --------- | ----------------------------------------------------------------- | ------------------------ |
| `create/` | Actual forward migrations: create or change schema.               | `001_create_members.sql` |
| `revert/` | Undo the paired create migration. Use the same migration number.  | `001_members_revert.sql` |
| `purify/` | Clear data from the added table while preserving its schema.      | `001_purify_members.sql` |
| `load/`   | Load fixtures using PostgreSQL SQL statements in the file itself. | `001_load_members.sql`   |

- Every new table requires a create migration, its matching revert script, and a
  purify script.
- Add a load script when fixtures are requested or needed. Keep fixture data
  directly in SQL, not in a separate JSON/CSV file or Rust seeder.
- Use the same numeric prefix across scripts belonging to one migration. Choose
  the next unused number, padded to at least three digits.
- Keep revert scripts paired with their forward migration; revert dependent
  migrations in reverse order.
- Purify means deleting table data, not dropping the table. Account for foreign
  keys without clearing unrelated tables implicitly.
- Never run revert, purify, or load scripts automatically on API startup. Only
  `sql/create/` contains forward migrations eligible for automatic execution.
- Do not modify an already applied migration; add a new numbered migration
  instead.

The API and `make db-migrate` both use SQLx migration tracking and read
`backend/sql/create/`. Do not maintain another forward-migration location.
Maintenance scripts run inside a transaction; do not add transaction control
statements to them. Purify scripts run in descending migration order; load
scripts run in ascending order. Revert scripts run only for applied migrations,
in descending order.
