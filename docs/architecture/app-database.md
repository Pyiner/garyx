# Garyx App Database

Garyx App Database is an agent-first, global SQLite database for lightweight
APaaS-style storage. Agents use it through `garyx db ...` CLI commands. The
Mac app can manage the same state later through the HTTP API.

## Decisions

- Storage is one global SQLite database under the Garyx data directory:
  `~/.garyx/data/app-database.sqlite3` by default.
- Tables are real SQLite tables, not JSON blob collections.
- Agent-visible table and column names are the actual SQLite names.
- Table and column names must be stable ASCII `snake_case` identifiers. Chinese
  or richer labels belong in `display_name`.
- User columns use only SQLite STRICT storage classes: `TEXT`, `INTEGER`,
  `REAL`, `BLOB`, and `ANY`.
- Garyx creates dynamic tables as SQLite `STRICT` tables.
- Every dynamic table has system columns:
  `id`, `created_at`, `updated_at`, `created_by`, `updated_by`.
- Read queries use SQL: `garyx db sql "select ..."` accepts only read-only
  SQLite statements. Write SQL is rejected by the gateway.
- Writes go through narrow API/CLI operations: create/drop table, add/drop
  field, insert/update/delete record.
- Data-change triggers are Automation triggers, not SQLite triggers and not
  database-owned behavior.

## System Tables

The service owns internal tables prefixed with `gx_db_`; user tables cannot use
that prefix.

- `gx_db_tables`: table registry and schema version.
- `gx_db_fields`: user column metadata.
- `gx_db_schema_versions`: schema snapshots for audit and recovery.
- `gx_db_events`: append-only data and schema event log.
- `gx_automation_data_triggers`: Automation data-trigger definitions.

## Event Model

Every schema and record mutation writes an event:

- `record.created`
- `record.updated`
- `record.deleted`
- `schema.changed`

Record events store `before` and `after` JSON snapshots where available.
Schema events store the schema snapshot in `after`.

## Automation Trigger Model

Garyx Automation has multiple trigger mechanisms. Scheduled automations are
time-triggered. App Database automations are data-triggered from `gx_db_events`.
Both mechanisms create Garyx tasks as the execution unit.

The first data-trigger implementation supports table/event triggers. The
definition contains:

- `table_name`
- `event_type`
- `title_template`
- `body_template`
- optional `agent_id`
- optional `workspace_dir`

Template placeholders are deliberately simple:

- `{event_type}`
- `{table_name}`
- `{record_id}`
- `{event_id}`

The App Database module only records events. The Automation module owns trigger
definition management, matching data events, template rendering, and task
creation.

## CLI Shape

Examples:

```bash
garyx db table create contacts \
  --display-name "Contacts" \
  --field name:TEXT \
  --field score:REAL \
  --field notes:TEXT

garyx db record insert contacts --data '{"name":"Test User","score":9.5}'
garyx db sql "select id, name, score from contacts order by created_at desc"

garyx automation trigger data create contacts record.created \
  --title "New contact: {record_id}" \
  --body "Review {table_name} record {record_id}" \
  --agent-id codex
```

The CLI is intentionally low-level. Agents are expected to inspect errors,
adjust SQL/schema, and retry.
