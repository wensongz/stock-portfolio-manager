# P0 Data Integrity Fix Design

## Scope

This change fixes two independent P0 defects discovered in the second simplification audit:

1. Editing or deleting a historical stock transaction derives the new holding from the current aggregate, so valid history edits can create negative positions or orphan SELL rows.
2. Options CSV import writes accepted rows one at a time and recomputes statuses outside the import operation, so a database error can leave a partial batch or stale statuses. Non-empty malformed numeric values are also silently converted to zero.

Quote synchronization, quarterly request races, matching-rule consolidation, dead APIs, and indexing remain outside this change.

## Transaction Mutation Design

Stock holding state must be a projection of the complete ordered transaction history for one `(account_id, symbol)` key. A new `position_replay` service will own the projection and database repair operations. It will:

- load non-cash transactions ordered by `traded_at`, then `created_at`, then `id`;
- replay `OPEN`, `BUY`, `SELL`, and `PAY` using the existing per-market cost policy;
- reject a SELL that would make the historical position negative;
- update one primary holding, relink all transactions for the key, and remove duplicate holding rows;
- create a holding only when the history contains an `OPEN` or `BUY` position-producing transaction;
- bulk-load all holdings and transactions for a full rebuild, avoiding one transaction query per holding.

`update_transaction` and `delete_transaction` will keep their caller-owned SQLite transaction. They will update or delete the transaction row, adjust the additive cash impact, and rebuild the affected old and new stock keys before committing. Any validation, replay, relinking, cash, or database failure rolls the entire mutation back.

The quote-provider configuration service will persist provider settings and, only when a cost-adjustment flag changes, run the full position rebuild in the same SQLite transaction. The frontend will stop issuing a second `recalculate_holdings_cost` command.

## Options Import Design

Options import retains partial acceptance for row-level input errors: malformed rows are reported and valid rows are imported. The distinction is that every accepted row and its derived contract statuses form one database transaction.

Numeric parsing will treat a blank optional amount, commission, or fee as zero, preserving existing exports. A non-empty value that cannot be parsed as a finite number becomes a row error. Quantity must be a non-zero integer. No malformed non-empty value is silently coerced to zero.

`recompute_option_statuses_in` will operate on a caller-provided connection and propagate every reset/update failure. The public recompute wrapper will use its own transaction; options import will call the connection-level function before committing its batch. A database failure in any accepted insert or status update rolls back the full accepted batch.

The existing split matching and read-time status recomputation remain unchanged here; their consolidation is a separate P1 change.

## Verification

Regression tests must prove:

- deleting an earlier BUY followed by a SELL is rejected and leaves the transaction, holding, and cash unchanged;
- reducing or moving a BUY cannot leave a negative or orphan SELL history;
- a valid historical edit rebuilds the final holding from chronological history;
- changing cost policy and rebuilding positions commits or rolls back as one unit;
- malformed option numerics are reported and not inserted;
- a forced failure on the second option insert rolls back the first insert;
- a forced status-update failure rolls back all imported option rows.

The final gate is `bun run check` plus `git diff --check`.
