# SQLite v2 Index Migration Design

## Problem

Frequent transaction and option queries filter or order by columns that have no supporting indexes. The database schema is still at version 1.

## Design

Bump `CURRENT_SCHEMA_VERSION` to 2 and add a sequential v2 migration after legacy transaction-table repairs. Create indexes for account/symbol/time transaction history, account/time transaction history, transaction `holding_id`, and account/contract/time option history. Use an expression index for normalized transaction symbols because production queries compare `UPPER(symbol)`.

Add the same indexes to fresh-schema creation so new and migrated databases converge. Index creation is idempotent and remains inside the migration transaction; `user_version` advances only after every statement succeeds.

## Verification

Migration tests will start from v1, assert version 2 and all index definitions, reopen the database, and confirm idempotence. Query-plan tests will verify representative transaction and option lookups use the intended indexes.
