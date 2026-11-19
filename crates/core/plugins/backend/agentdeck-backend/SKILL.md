---
name: agentdeck-backend
description: Use for API endpoints, server actions, database schema or ORM migrations, query design, queues and external integrations. Check authorization, transactions, retries and idempotent handling of duplicate webhook deliveries.
---

# Backend and data

## Contract first

Write the contract before the code: method and path (or function signature), input and output types
with field names, every error with its status, who may call it. Reuse the project's existing style
(REST, RPC, server actions, GraphQL); one change of a contract updates its callers in the same change.

## Endpoint checklist

- **Validate** every input at the boundary with the project's schema tool (zod, valibot, pydantic,
  serde, Joi): types, ranges, lengths, enums. Reject unknown fields on writes.
- **Authorize per resource**, not only per route: load the object, check it belongs to the caller
  (`WHERE id = $1 AND owner_id = $2`). Never trust an id, role or price sent by the client.
- **Errors**: one shape across the API (`{ error: { code, message } }` or the project's), correct
  status (400 input, 401 no auth, 403 forbidden, 404, 409 conflict, 422, 429), generic message to the
  client, details and a request id in the logs.
- **Writes**: in a transaction when they touch several rows or tables; idempotent when a client or a
  webhook may retry (idempotency key, upsert, unique constraint); money in integer cents.
- **Lists**: paginated (cursor for large or live data), a maximum page size, a stable order.
- **External calls**: timeout always, retry only idempotent calls with backoff and jitter, never call
  a slow service inside a database transaction.
- **Secrets** from env or a secret store, never in code, logs or error messages.

## Database

- Every query that runs per request uses an index: check with `EXPLAIN` (Postgres `EXPLAIN ANALYZE`,
  SQLite `EXPLAIN QUERY PLAN`) on realistic data.
- No N+1: one query with a join or `IN (...)`, or the ORM's eager loading. A query in a loop is a bug.
- Constraints in the database, not only in code: `NOT NULL`, `UNIQUE`, foreign keys, `CHECK`.
- Parameterized queries only; never build SQL with string concatenation.

## Migrations (expand, migrate, contract)

1. **Expand**: add the new column or table, nullable or with a default; deploy code that writes both.
2. **Migrate**: backfill in batches; verify counts.
3. **Contract**: switch reads, then drop the old column in a later migration.

- Never edit a migration that has run anywhere; add a new one.
- Large tables: create indexes concurrently (Postgres `CREATE INDEX CONCURRENTLY`), avoid long locks.
- Each migration runs on a fresh database and on a copy of current data; know how to roll back.

## Background jobs and webhooks

Jobs are idempotent and retryable with a limit, record their state, and do not hold a transaction
while calling out. Webhooks verify the signature, answer fast (enqueue the work), and dedupe by event id.

## Prove it

- Run the migration on a fresh database, then the tests.
- Call each changed endpoint for real (curl, httpie, the app in the AgentDeck browser panel): the happy
  path, invalid input, another user's resource (expect 403/404), a retry of the same write.
- Read the server log for the request: no stack trace, no secret, a request id.

Report: contract changes, migrations and how they were run, requests made with their status codes.
