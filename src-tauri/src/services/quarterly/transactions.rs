use super::*;

/// Return all transactions within the quarter of the given snapshot, grouped by
/// stock symbol. OPEN-type transactions (initial position entries) are excluded.
pub fn get_quarterly_transactions(
    db: &Database,
    snapshot_id: &str,
) -> Result<Vec<StockTransactionGroup>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Fetch the quarter string for this snapshot
    let quarter: String = conn
        .query_row(
            "SELECT quarter FROM quarterly_snapshots WHERE id = ?1",
            rusqlite::params![snapshot_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Snapshot not found: {}", e))?;

    let (year, q) = parse_quarter(&quarter)?;
    let start = quarter_start_date(year, q);
    let end = quarter_end_date(year, q);

    // ISO-8601 date strings used for comparison with the traded_at column
    let start_str = start.format("%Y-%m-%d").to_string();
    // Include the full end day by using the day after as an exclusive upper bound
    let end_exclusive = end.succ_opt().unwrap_or(end);
    let end_str = end_exclusive.format("%Y-%m-%d").to_string();

    // Fetch all real transactions in the quarter date range, ordered by date.
    // Exclude OPEN-type records (initial position entries) and backfill imports
    // (notes = 'backfill:initial') which are also synthetic rather than real trades.
    let mut stmt = conn
        .prepare(
            "SELECT id, holding_id, account_id, symbol, name, market, transaction_type,
                    shares, price, total_amount, commission, currency, traded_at, notes, created_at
             FROM transactions
             WHERE transaction_type != 'OPEN'
               AND (notes IS NULL OR notes != 'backfill:initial')
               AND symbol NOT LIKE '$CASH-%'
               AND DATE(traded_at) >= ?1
               AND DATE(traded_at) < ?2
             ORDER BY CASE market WHEN 'CN' THEN 1 WHEN 'HK' THEN 2 ELSE 3 END, symbol ASC, traded_at ASC",
        )
        .map_err(|e| e.to_string())?;

    let txns: Vec<Transaction> = stmt
        .query_map(rusqlite::params![start_str, end_str], |row| {
            Ok(Transaction {
                id: row.get(0)?,
                holding_id: row.get(1)?,
                account_id: row.get(2)?,
                symbol: row.get(3)?,
                name: row.get(4)?,
                market: row.get(5)?,
                transaction_type: row.get(6)?,
                shares: row.get(7)?,
                price: row.get(8)?,
                total_amount: row.get(9)?,
                commission: row.get(10)?,
                currency: row.get(11)?,
                traded_at: row.get(12)?,
                notes: row.get(13)?,
                created_at: row.get(14)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // Group by symbol, preserving insertion order (symbols are already sorted)
    let mut groups: Vec<StockTransactionGroup> = Vec::new();
    let mut symbol_index: HashMap<String, usize> = HashMap::new();

    for txn in txns {
        let idx = if let Some(&i) = symbol_index.get(&txn.symbol) {
            i
        } else {
            let i = groups.len();
            groups.push(StockTransactionGroup {
                symbol: txn.symbol.clone(),
                name: txn.name.clone(),
                market: txn.market.clone(),
                currency: txn.currency.clone(),
                buy_count: 0,
                sell_count: 0,
                total_buy_shares: 0.0,
                total_sell_shares: 0.0,
                total_buy_amount: 0.0,
                total_sell_amount: 0.0,
                transactions: Vec::new(),
            });
            symbol_index.insert(txn.symbol.clone(), i);
            i
        };

        let g = &mut groups[idx];
        match txn.transaction_type.as_str() {
            "BUY" => {
                g.buy_count += 1;
                g.total_buy_shares += txn.shares;
                g.total_buy_amount += txn.total_amount;
            }
            "SELL" => {
                g.sell_count += 1;
                g.total_sell_shares += txn.shares;
                g.total_sell_amount += txn.total_amount;
            }
            _ => {}
        }
        g.transactions.push(txn);
    }

    Ok(groups)
}
