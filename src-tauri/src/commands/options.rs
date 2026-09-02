use crate::db::Database;
use crate::models::option::{
    CallContractSimulation, ExpiredOptionStats, OptionContract, OptionRecord,
    PutContractSimulation, SellCallSimulation, SellPutSimulation,
};
use crate::models::stock_split::StockSplit;
use tauri::State;
use tracing::warn;

/// Parse the option symbol like "PDD 20FEB26 100 P" or "BRK B 16JUN23 330 C" into components.
/// Returns (underlying, expiry_date, strike_price, option_type)
/// Supports multi-word tickers (e.g. "BRK B") by parsing from the end.
fn parse_option_symbol(symbol: &str) -> Result<(String, String, f64, String), String> {
    let parts: Vec<&str> = symbol.split_whitespace().collect();
    if parts.len() < 4 {
        return Err(format!("Invalid option symbol: {}", symbol));
    }

    // Parse from the end: last part is option_type, second-to-last is strike, third-to-last is expiry
    // Everything before that is the underlying ticker (handles multi-word tickers like "BRK B")
    let len = parts.len();
    let option_type = parts[len - 1].to_string();
    if option_type != "P" && option_type != "C" {
        return Err(format!(
            "Invalid option type '{}' in: {}",
            option_type, symbol
        ));
    }
    let strike_price: f64 = parts[len - 2]
        .parse()
        .map_err(|_| format!("Invalid strike price in: {}", symbol))?;
    let expiry_date = parts[len - 3].to_string();
    let underlying = parts[..len - 3].join(" ");
    if underlying.is_empty() {
        return Err(format!("Invalid option symbol: {}", symbol));
    }
    Ok((underlying, expiry_date, strike_price, option_type))
}

/// Import option records from CSV content.
/// CSV columns: 账户, 股票, 交易时间, 交割时间, 交易所, 操作, 股票数量, 价格, 金额, 佣金, 费用, 类型, 代码
/// English (IBKR-style) equivalents are also accepted:
/// Acct ID, Symbol, Trade Date/Time, Settle Date, Exchange, Type, Quantity,
/// Price, Proceeds, Comm, Fee, Order Type, Code.
/// Header matching is case-insensitive.
///
/// Boundary check: close records (BUY with code C / C;Ep / A;C / C;P — closed, expired,
/// assigned or closed) must have a matching open record (SELL with code
/// starting with "O") with enough remaining quantity, either already in the
/// database or in the same CSV, or cross-symbol via a configured stock split.
/// Close records that cannot be matched are rejected instead of inserted, so
/// option_records never gains an inconsistent orphan close.
#[tauri::command(rename_all = "camelCase")]
pub fn import_options_csv(
    db: State<Database>,
    account_id: String,
    csv_content: String,
) -> Result<ImportOptionsResult, String> {
    import_options_csv_inner(&db, &account_id, &csv_content)
}

/// A row parsed from the CSV, validated but not yet written.
struct ParsedOptionRow {
    row_num: usize,
    option_symbol: String,
    underlying: String,
    expiry_date: String,
    strike_price: f64,
    option_type: String,
    action: String,
    code: String,
    quantity: i64,
    price: f64,
    amount: f64,
    commission: f64,
    fee: f64,
    traded_at: Option<String>,
    settled_at: Option<String>,
}

/// Minimal open-record details needed for cross-symbol split matching.
struct OpenDetail {
    underlying: String,
    expiry_date: String,
    strike_price: f64,
    option_type: String,
}

/// Whether a close record can be matched cross-symbol to an open record via a
/// configured stock split (mirrors the orphan-close matching in
/// `recompute_option_statuses`).
fn has_split_match(
    opens: &[OpenDetail],
    splits: &[StockSplit],
    close_underlying: &str,
    close_expiry: &str,
    close_strike: f64,
    close_type: &str,
) -> bool {
    for open in opens {
        if open.underlying != close_underlying
            || open.expiry_date != close_expiry
            || open.option_type != close_type
        {
            continue;
        }
        for split in splits {
            if split.stock_code != open.underlying {
                continue;
            }
            let split_ymd = match parse_split_ymd(&split.split_date) {
                Some(d) => d,
                None => continue,
            };
            let exp_ymd = match parse_expiry_ymd(close_expiry) {
                Some(d) => d,
                None => continue,
            };
            if (split_ymd.0, split_ymd.1, split_ymd.2) > (exp_ymd.0, exp_ymd.1, exp_ymd.2) {
                continue;
            }
            let ratio = split.ratio_to as f64 / split.ratio_from as f64;
            let expected_strike = open.strike_price / ratio;
            if expected_strike > 0.0 {
                let strike_diff = (close_strike - expected_strike).abs() / expected_strike;
                if strike_diff <= 0.02 {
                    return true;
                }
            }
        }
    }
    false
}

/// Internal helper without the tauri::State wrapper (testable directly).
/// See `import_options_csv` for the CSV format.
fn import_options_csv_inner(
    db: &Database,
    account_id: &str,
    csv_content: &str,
) -> Result<ImportOptionsResult, String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Strip UTF-8 BOM if present
    let content = csv_content.strip_prefix('\u{feff}').unwrap_or(csv_content);

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers = reader
        .headers()
        .map_err(|e| format!("Failed to read CSV headers: {}", e))?
        .clone();

    let mut imported = 0;
    let mut skipped = 0;
    let mut errors: Vec<String> = Vec::new();

    // ---- Pass 1: parse and validate every row (no DB writes yet) ----
    let mut parsed: Vec<ParsedOptionRow> = Vec::new();

    for (i, result) in reader.records().enumerate() {
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("Row {}: {}", i + 2, e));
                continue;
            }
        };

        // Skip "Total" summary rows
        let first_field = record.get(0).unwrap_or("").trim();
        if first_field.starts_with("Total")
            || first_field.starts_with("总数")
            || first_field.is_empty()
        {
            skipped += 1;
            continue;
        }

        // Get the option symbol (column index 1: 股票)
        let option_symbol = match get_field(
            &record,
            &headers,
            &[
                "股票",
                "股票代码",
                "合约",
                "期权",
                "期权代码",
                "symbol",
                "Symbol",
            ],
        ) {
            Some(s) if !s.is_empty() => s,
            _ => {
                skipped += 1;
                continue;
            }
        };

        // Parse option symbol
        let (underlying, expiry_date, strike_price, option_type) =
            match parse_option_symbol(&option_symbol) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("Row {}: {}", i + 2, e));
                    continue;
                }
            };

        // Parse other fields
        let action_raw = get_field(
            &record,
            &headers,
            &["操作", "买/卖", "买卖", "action", "Action", "Type"],
        )
        .unwrap_or_default();
        let action = normalize_action(&action_raw);
        if action.is_empty() {
            errors.push(format!("Row {}: invalid action '{}'", i + 2, action_raw));
            continue;
        }

        let code = get_field(&record, &headers, &["代码", "code", "Code"]).unwrap_or_default();
        let quantity_str = get_field(
            &record,
            &headers,
            &[
                "股票数量",
                "数量",
                "合约数量",
                "合约数",
                "quantity",
                "Quantity",
            ],
        )
        .unwrap_or_default();
        let quantity = match parse_quantity(&quantity_str) {
            Ok(quantity) => quantity,
            Err(error) => {
                errors.push(format!("Row {}: {}", i + 2, error));
                continue;
            }
        };

        let price_str =
            get_field(&record, &headers, &["价格", "price", "Price"]).unwrap_or_default();
        let price = match parse_required_decimal(&price_str, "price") {
            Ok(price) => price,
            Err(error) => {
                errors.push(format!("Row {}: {}", i + 2, error));
                continue;
            }
        };

        let amount_str = get_field(&record, &headers, &["金额", "amount", "Amount", "Proceeds"])
            .unwrap_or_default();
        let amount = match parse_decimal(&amount_str, "amount") {
            Ok(amount) => amount,
            Err(error) => {
                errors.push(format!("Row {}: {}", i + 2, error));
                continue;
            }
        };

        let commission_str = get_field(
            &record,
            &headers,
            &["佣金", "commission", "Commission", "Comm"],
        )
        .unwrap_or_default();
        let commission = match parse_decimal(&commission_str, "commission") {
            Ok(commission) => commission,
            Err(error) => {
                errors.push(format!("Row {}: {}", i + 2, error));
                continue;
            }
        };

        let fee_str = get_field(&record, &headers, &["费用", "fee", "Fee"]).unwrap_or_default();
        let fee = match parse_decimal(&fee_str, "fee") {
            Ok(fee) => fee,
            Err(error) => {
                errors.push(format!("Row {}: {}", i + 2, error));
                continue;
            }
        };

        let traded_at = get_field(
            &record,
            &headers,
            &[
                "交易时间",
                "traded_at",
                "Trade Date",
                "Trade Date/Time",
                "Date/Time",
            ],
        );
        let settled_at = get_field(
            &record,
            &headers,
            &["交割时间", "settled_at", "Settle Date"],
        );

        parsed.push(ParsedOptionRow {
            row_num: i + 2,
            option_symbol,
            underlying,
            expiry_date,
            strike_price,
            option_type,
            action,
            code,
            quantity,
            price,
            amount,
            commission,
            fee,
            traded_at,
            settled_at,
        });
    }

    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;

    // ---- Boundary check: load existing DB state ----
    // Remaining open quantity per option symbol (open qty minus close qty).
    // Only symbols with remaining > 0 can back an imported close record.
    let mut available_by_symbol: std::collections::HashMap<String, i64> = {
        let mut stmt = transaction
            .prepare(
                "SELECT option_symbol,
                        COALESCE(SUM(CASE WHEN action = 'SELL' AND code LIKE 'O%' THEN ABS(quantity) ELSE 0 END), 0)
                      - COALESCE(SUM(CASE WHEN action = 'BUY' AND code IN ('C', 'C;Ep', 'A;C', 'C;P') THEN ABS(quantity) ELSE 0 END), 0)
                 FROM option_records WHERE account_id = ?1
                 GROUP BY option_symbol",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![account_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (symbol, remaining) = row.map_err(|e| e.to_string())?;
            if remaining > 0 {
                map.insert(symbol, remaining);
            }
        }
        map
    };

    // Opens in the DB (SELL + O*) whose group still has remaining quantity,
    // eligible for cross-symbol split matching.
    let mut open_pool: Vec<OpenDetail> = {
        let mut stmt = transaction
            .prepare(
                "SELECT option_symbol, underlying, expiry_date, strike_price, option_type
                 FROM option_records
                 WHERE account_id = ?1 AND action = 'SELL' AND code LIKE 'O%'",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![account_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    OpenDetail {
                        underlying: row.get(1)?,
                        expiry_date: row.get(2)?,
                        strike_price: row.get(3)?,
                        option_type: row.get(4)?,
                    },
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut opens = Vec::new();
        for row in rows {
            let (option_symbol, open) = row.map_err(|e| e.to_string())?;
            if available_by_symbol.contains_key(&option_symbol) {
                opens.push(open);
            }
        }
        opens
    };

    // Stock splits config, used for cross-symbol matching of split-affected contracts
    let splits: Vec<StockSplit> = {
        let mut stmt = transaction
            .prepare(
                "SELECT id, stock_code, split_date, ratio_from, ratio_to, created_at
                 FROM stock_splits",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(StockSplit {
                    id: row.get(0)?,
                    stock_code: row.get(1)?,
                    split_date: row.get(2)?,
                    ratio_from: row.get(3)?,
                    ratio_to: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        result
    };

    // Opens in the same CSV participate in the boundary check too, regardless
    // of their order relative to close records in the file.
    for row in &parsed {
        if row.action == "SELL" && row.code.starts_with("O") {
            // Exported CSVs carry negative quantities for SELL opens; the
            // boundary check works with positive contract counts.
            *available_by_symbol
                .entry(row.option_symbol.clone())
                .or_insert(0) += row.quantity.abs();
            open_pool.push(OpenDetail {
                underlying: row.underlying.clone(),
                expiry_date: row.expiry_date.clone(),
                strike_price: row.strike_price,
                option_type: row.option_type.clone(),
            });
        }
    }

    // ---- Pass 2: boundary check for close records, then insert ----
    for row in &parsed {
        let is_close = row.action == "BUY" && is_close_code(&row.code);
        if is_close {
            let close_qty = row.quantity.abs();
            let avail = available_by_symbol
                .get(&row.option_symbol)
                .copied()
                .unwrap_or(0);
            if avail >= close_qty {
                available_by_symbol.insert(row.option_symbol.clone(), avail - close_qty);
            } else if !has_split_match(
                &open_pool,
                &splits,
                &row.underlying,
                &row.expiry_date,
                row.strike_price,
                &row.option_type,
            ) {
                errors.push(format!(
                    "Row {}: close record {} ({}) has no matching open record; skipped. \
                     If the contract was split-adjusted, configure the split info in Settings first",
                    row.row_num, row.option_symbol, row.code
                ));
                continue;
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        transaction.execute(
            "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, 'active')",
            rusqlite::params![
                id,
                account_id,
                row.option_symbol,
                row.underlying,
                row.expiry_date,
                row.strike_price,
                row.option_type,
                row.action,
                row.code,
                row.quantity,
                row.price,
                row.amount,
                row.commission,
                row.fee,
                row.traded_at,
                row.settled_at,
                now,
            ],
        )
        .map_err(|e| format!("Row {}: {}", row.row_num, e))?;

        imported += 1;
    }

    if !errors.is_empty() {
        warn!("[期权导入] 错误 {} 条:", errors.len());
        for e in &errors {
            warn!("  - {}", e);
        }
    }

    // Recompute statuses before committing so any DB failure rolls back the
    // entire accepted subset of this import.
    if imported > 0 {
        recompute_option_statuses_in(&transaction, account_id)?;
    }
    transaction.commit().map_err(|e| e.to_string())?;

    Ok(ImportOptionsResult {
        imported,
        skipped,
        errors,
    })
}

/// Recompute contract_status for all open records of an account.
/// Pairs open (SELL+O) and close (BUY+C/C;Ep/A;C/C;P) records by option_symbol,
/// and handles cross-symbol split-affected contract matching.
fn recompute_option_statuses(db: &Database, account_id: &str) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    recompute_option_statuses_in(&transaction, account_id)?;
    transaction.commit().map_err(|e| e.to_string())
}

fn recompute_option_statuses_in(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<(), String> {
    // Load full records for this account
    let mut stmt = conn
        .prepare(
            "SELECT id, option_symbol, underlying, expiry_date, strike_price,
                    option_type, action, code, quantity, price, traded_at
             FROM option_records WHERE account_id = ?1
             ORDER BY option_symbol, traded_at",
        )
        .map_err(|e| e.to_string())?;

    struct Rec {
        id: String,
        option_symbol: String,
        underlying: String,
        expiry_date: String,
        strike_price: f64,
        option_type: String,
        action: String,
        code: String,
        quantity: i64,
        traded_at: Option<String>,
    }

    let records: Vec<Rec> = stmt
        .query_map(rusqlite::params![account_id], |row| {
            Ok(Rec {
                id: row.get(0)?,
                option_symbol: row.get(1)?,
                underlying: row.get(2)?,
                expiry_date: row.get(3)?,
                strike_price: row.get(4)?,
                option_type: row.get(5)?,
                action: row.get(6)?,
                code: row.get(7)?,
                quantity: row.get(8)?,
                traded_at: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // Reset all contract_status to 'active' first
    conn.execute(
        "UPDATE option_records SET contract_status = 'active' WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| e.to_string())?;

    // Group by option_symbol
    let mut groups: std::collections::HashMap<String, Vec<&Rec>> = std::collections::HashMap::new();
    for r in &records {
        groups.entry(r.option_symbol.clone()).or_default().push(r);
    }

    // Track orphan closes (groups with closes but no opens — potential split-affected)
    let mut orphan_closes: Vec<&Rec> = Vec::new();

    // Phase 1: same-symbol matching
    for group_recs in groups.values() {
        let mut opens: Vec<&Rec> = group_recs
            .iter()
            .filter(|r| r.action == "SELL" && r.code.starts_with("O"))
            .copied()
            .collect();
        opens.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));

        let mut closes: Vec<&Rec> = group_recs
            .iter()
            .filter(|r| r.action == "BUY" && is_close_code(&r.code))
            .copied()
            .collect();
        closes.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));

        if opens.is_empty() {
            if !closes.is_empty() {
                for c in &closes {
                    orphan_closes.push(c);
                }
            }
            continue;
        }

        let mut remaining_by_open: Vec<_> = opens
            .iter()
            .map(|open| (open.id.as_str(), open.quantity.abs()))
            .collect();

        for close in closes {
            let mut remaining_close = close.quantity.abs();
            for (open_id, remaining_open) in &mut remaining_by_open {
                if remaining_close == 0 {
                    break;
                }
                if *remaining_open == 0 {
                    continue;
                }

                let matched = (*remaining_open).min(remaining_close);
                *remaining_open -= matched;
                remaining_close -= matched;

                if *remaining_open == 0 {
                    let status = match close.code.as_str() {
                        "A;C" => "assigned",
                        "C;P" | "C" => "closed",
                        _ => "expired",
                    };
                    conn.execute(
                        "UPDATE option_records SET contract_status = ?1 WHERE id = ?2",
                        rusqlite::params![status, *open_id],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // Phase 2: cross-symbol split matching
    if !orphan_closes.is_empty() {
        // Load stock splits
        let splits: Vec<StockSplit> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, stock_code, split_date, ratio_from, ratio_to, created_at
                     FROM stock_splits",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(StockSplit {
                        id: row.get(0)?,
                        stock_code: row.get(1)?,
                        split_date: row.get(2)?,
                        ratio_from: row.get(3)?,
                        ratio_to: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row.map_err(|e| e.to_string())?);
            }
            result
        };

        if !splits.is_empty() {
            // Active open records that haven't been matched yet
            // (those still with contract_status = 'active' after Phase 1)
            let active_open_ids: Vec<String> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT id FROM option_records
                         WHERE account_id = ?1 AND action = 'SELL' AND code LIKE 'O%'
                           AND contract_status = 'active'",
                    )
                    .map_err(|e| e.to_string())?;
                let ids = stmt
                    .query_map(rusqlite::params![account_id], |row| row.get::<_, String>(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                ids
            };
            let active_opens: Vec<&Rec> = records
                .iter()
                .filter(|r| {
                    r.action == "SELL" && r.code.starts_with("O") && active_open_ids.contains(&r.id)
                })
                .collect();

            for ao in &active_opens {
                // Check if already matched (contract_status changed from 'active')
                // We need to re-read status; for now check all active opens
                'split_loop: for split in &splits {
                    if split.stock_code != ao.underlying {
                        continue;
                    }
                    let split_ymd = match parse_split_ymd(&split.split_date) {
                        Some(d) => d,
                        None => continue,
                    };
                    let exp_ymd = match parse_expiry_ymd(&ao.expiry_date) {
                        Some(d) => d,
                        None => continue,
                    };
                    if (split_ymd.0, split_ymd.1, split_ymd.2) > (exp_ymd.0, exp_ymd.1, exp_ymd.2) {
                        continue;
                    }

                    let ratio = split.ratio_to as f64 / split.ratio_from as f64;
                    let expected_strike = ao.strike_price / ratio;

                    // Find matching orphan closes
                    let mut matched_qty: i64 = 0;
                    let mut last_code: Option<&str> = None;
                    let contract_qty = ao.quantity.abs();

                    for oc in &orphan_closes {
                        if oc.underlying != ao.underlying
                            || oc.expiry_date != ao.expiry_date
                            || oc.option_type != ao.option_type
                        {
                            continue;
                        }
                        let strike_diff = if expected_strike > 0.0 {
                            (oc.strike_price - expected_strike).abs() / expected_strike
                        } else {
                            1.0
                        };
                        if strike_diff <= 0.02 {
                            matched_qty += oc.quantity.abs();
                            last_code = Some(oc.code.as_str());
                        }
                    }

                    if matched_qty >= contract_qty {
                        let status = match last_code {
                            Some("A;C") => "assigned",
                            Some("C;P") | Some("C") => "closed",
                            _ => "expired",
                        };
                        conn.execute(
                            "UPDATE option_records SET contract_status = ?1 WHERE id = ?2",
                            rusqlite::params![status, ao.id],
                        )
                        .map_err(|e| e.to_string())?;
                        break 'split_loop;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Get all option contracts for an account, paired by option_symbol
#[tauri::command(rename_all = "camelCase")]
pub fn get_option_contracts(
    db: State<Database>,
    account_id: String,
) -> Result<Vec<OptionContract>, String> {
    get_option_contracts_inner(&db, &account_id)
}

/// Get statistics for expired options
#[tauri::command(rename_all = "camelCase")]
pub fn get_expired_option_stats(
    db: State<Database>,
    account_id: String,
) -> Result<ExpiredOptionStats, String> {
    let contracts = get_option_contracts_inner(&db, &account_id)?;

    let expired: Vec<&OptionContract> = contracts.iter().filter(|c| c.status != "active").collect();
    let total = expired.len() as i64;
    let assigned = expired
        .iter()
        .filter(|c| c.close_code.as_deref() == Some("A;C"))
        .count() as i64;
    let expired_count = expired
        .iter()
        .filter(|c| c.close_code.as_deref() == Some("C;Ep"))
        .count() as i64;

    let ratio = if total > 0 {
        assigned as f64 / total as f64
    } else {
        0.0
    };

    Ok(ExpiredOptionStats {
        total_contracts: total,
        assigned_contracts: assigned,
        expired_contracts: expired_count,
        assignment_ratio: ratio,
    })
}

/// Simulate sell put assignments given stock prices
#[tauri::command(rename_all = "camelCase")]
pub fn simulate_sell_put(
    db: State<Database>,
    account_id: String,
    stock_prices: Vec<StockPriceInput>,
) -> Result<Vec<SellPutSimulation>, String> {
    let contracts = get_option_contracts_inner(&db, &account_id)?;

    // Load share lot sizes (default 100 if not configured)
    let share_lots: std::collections::HashMap<String, i64> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT stock_code, shares_per_contract FROM option_share_lots")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (code, shares) = row.map_err(|e| e.to_string())?;
            map.insert(code.to_uppercase(), shares);
        }
        map
    };

    let get_shares = |underlying: &str| -> f64 {
        share_lots
            .get(&underlying.to_uppercase())
            .copied()
            .unwrap_or(100) as f64
    };

    let active_puts: Vec<&OptionContract> = contracts
        .iter()
        .filter(|c| c.status == "active" && c.option_type == "P")
        .collect();

    // Group by underlying
    let mut grouped: std::collections::HashMap<String, Vec<&OptionContract>> =
        std::collections::HashMap::new();
    for contract in &active_puts {
        grouped
            .entry(contract.underlying.clone())
            .or_default()
            .push(contract);
    }

    let price_map: std::collections::HashMap<String, f64> = stock_prices
        .into_iter()
        .map(|sp| (sp.symbol.to_uppercase(), sp.price))
        .collect();

    let mut results: Vec<SellPutSimulation> = Vec::new();

    for (underlying, puts) in &grouped {
        let stock_price = price_map.get(&underlying.to_uppercase()).copied();
        let shares_per_contract = get_shares(underlying);

        let mut sim_contracts: Vec<PutContractSimulation> = Vec::new();
        let mut total_cash = 0.0;

        for put in puts {
            let would_be_assigned = match stock_price {
                Some(price) => price < put.strike_price,
                None => false,
            };
            let cash_needed = if would_be_assigned {
                put.strike_price * put.contracts.abs() as f64 * shares_per_contract
            } else {
                0.0
            };
            total_cash += cash_needed;

            sim_contracts.push(PutContractSimulation {
                option_symbol: put.option_symbol.clone(),
                strike_price: put.strike_price,
                contracts: put.contracts,
                would_be_assigned,
                cash_needed,
            });
        }

        results.push(SellPutSimulation {
            underlying: underlying.clone(),
            contracts: sim_contracts,
            total_cash_needed: total_cash,
        });
    }

    results.sort_by(|a, b| a.underlying.cmp(&b.underlying));
    Ok(results)
}

/// Simulate sell call assignments given stock prices
#[tauri::command(rename_all = "camelCase")]
pub fn simulate_sell_call(
    db: State<Database>,
    account_id: String,
    stock_prices: Vec<StockPriceInput>,
) -> Result<Vec<SellCallSimulation>, String> {
    let contracts = get_option_contracts_inner(&db, &account_id)?;

    // Load share lot sizes (default 100 if not configured)
    let share_lots: std::collections::HashMap<String, i64> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT stock_code, shares_per_contract FROM option_share_lots")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (code, shares) = row.map_err(|e| e.to_string())?;
            map.insert(code.to_uppercase(), shares);
        }
        map
    };

    let get_shares = |underlying: &str| -> i64 {
        share_lots
            .get(&underlying.to_uppercase())
            .copied()
            .unwrap_or(100)
    };

    let active_calls: Vec<&OptionContract> = contracts
        .iter()
        .filter(|c| c.status == "active" && c.option_type == "C")
        .collect();

    let mut grouped: std::collections::HashMap<String, Vec<&OptionContract>> =
        std::collections::HashMap::new();
    for contract in &active_calls {
        grouped
            .entry(contract.underlying.clone())
            .or_default()
            .push(contract);
    }

    let price_map: std::collections::HashMap<String, f64> = stock_prices
        .into_iter()
        .map(|sp| (sp.symbol.to_uppercase(), sp.price))
        .collect();

    let mut results: Vec<SellCallSimulation> = Vec::new();

    for (underlying, calls) in &grouped {
        let stock_price = price_map.get(&underlying.to_uppercase()).copied();
        let shares_per_contract = get_shares(underlying);

        let mut sim_contracts: Vec<CallContractSimulation> = Vec::new();
        let mut total_shares: i64 = 0;

        for call in calls {
            let would_be_assigned = match stock_price {
                Some(price) => price > call.strike_price,
                None => false,
            };
            let shares_needed = if would_be_assigned {
                call.contracts.abs() * shares_per_contract
            } else {
                0
            };
            total_shares += shares_needed;

            sim_contracts.push(CallContractSimulation {
                option_symbol: call.option_symbol.clone(),
                strike_price: call.strike_price,
                contracts: call.contracts,
                would_be_assigned,
                shares_needed,
            });
        }

        results.push(SellCallSimulation {
            underlying: underlying.clone(),
            contracts: sim_contracts,
            total_shares_needed: total_shares,
        });
    }

    results.sort_by(|a, b| a.underlying.cmp(&b.underlying));
    Ok(results)
}

/// Delete all option records for an account
#[tauri::command(rename_all = "camelCase")]
pub fn delete_option_records(db: State<Database>, account_id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM option_records WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Export option records as CSV string.
/// The output CSV uses the same format as the import CSV for round-trip compatibility.
#[tauri::command(rename_all = "camelCase")]
pub fn export_options_csv(db: State<Database>, account_id: String) -> Result<String, String> {
    export_options_csv_inner(&db, &account_id)
}

/// Internal helper without the tauri::State wrapper (testable directly).
fn export_options_csv_inner(db: &Database, account_id: &str) -> Result<String, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT option_symbol, traded_at, settled_at, action, quantity, price, amount, commission, fee, code
             FROM option_records WHERE account_id = ?1
             ORDER BY option_symbol, traded_at",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,         // option_symbol
                row.get::<_, Option<String>>(1)?, // traded_at
                row.get::<_, Option<String>>(2)?, // settled_at
                row.get::<_, String>(3)?,         // action
                row.get::<_, i64>(4)?,            // quantity
                row.get::<_, f64>(5)?,            // price
                row.get::<_, f64>(6)?,            // amount
                row.get::<_, f64>(7)?,            // commission
                row.get::<_, f64>(8)?,            // fee
                row.get::<_, String>(9)?,         // code
            ))
        })
        .map_err(|e| e.to_string())?;

    // Use csv::Writer for proper quoting of fields containing commas (e.g. traded_at)
    let mut wtr = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());

    // Write header row
    wtr.write_record([
        "股票",
        "交易时间",
        "交割时间",
        "操作",
        "股票数量",
        "价格",
        "金额",
        "佣金",
        "费用",
        "代码",
    ])
    .map_err(|e| e.to_string())?;

    for row in rows {
        let (symbol, traded_at, settled_at, action, quantity, price, amount, commission, fee, code) =
            row.map_err(|e| e.to_string())?;
        wtr.write_record(&[
            symbol,
            traded_at.unwrap_or_default(),
            settled_at.unwrap_or_default(),
            action,
            quantity.to_string(),
            format!("{:.2}", price),
            format!("{:.2}", amount),
            format!("{:.2}", commission),
            format!("{:.2}", fee),
            code,
        ])
        .map_err(|e| e.to_string())?;
    }

    let csv = String::from_utf8(wtr.into_inner().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(csv)
}

/// Parse options CSV and return a preview without importing.
/// This is used by the Import/Export page wizard.
#[tauri::command(rename_all = "camelCase")]
pub fn parse_options_csv(
    csv_content: String,
) -> Result<crate::models::import_export::ImportPreview, String> {
    use crate::models::import_export::{ImportError, ImportPreview};
    use std::collections::HashMap;

    let content = csv_content.strip_prefix('\u{feff}').unwrap_or(&csv_content);

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers = reader
        .headers()
        .map_err(|e| format!("Failed to read CSV headers: {}", e))?
        .clone();

    let mut total_rows: usize = 0;
    let mut valid_rows: usize = 0;
    let mut error_rows: Vec<ImportError> = Vec::new();
    let mut preview_data: Vec<serde_json::Value> = Vec::new();

    // Build column mapping from detected headers
    let mut column_mapping: HashMap<String, String> = HashMap::new();
    for h in headers.iter() {
        let trimmed = h.trim().to_string();
        column_mapping.insert(trimmed.clone(), trimmed);
    }

    for (i, result) in reader.records().enumerate() {
        total_rows += 1;
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                error_rows.push(ImportError {
                    row: i + 2,
                    column: "".to_string(),
                    message: format!("Parse error: {}", e),
                });
                continue;
            }
        };

        // Skip "Total" summary rows and empty rows
        let first_field = record.get(0).unwrap_or("").trim();
        if first_field.starts_with("Total")
            || first_field.starts_with("总数")
            || first_field.is_empty()
        {
            continue;
        }

        // Validate option symbol
        let option_symbol = match get_field(
            &record,
            &headers,
            &[
                "股票",
                "股票代码",
                "合约",
                "期权",
                "期权代码",
                "symbol",
                "Symbol",
            ],
        ) {
            Some(s) if !s.is_empty() => s,
            _ => {
                error_rows.push(ImportError {
                    row: i + 2,
                    column: "股票".to_string(),
                    message: "Missing option symbol".to_string(),
                });
                continue;
            }
        };

        // Parse option symbol to validate
        if parse_option_symbol(&option_symbol).is_err() {
            error_rows.push(ImportError {
                row: i + 2,
                column: "股票".to_string(),
                message: format!("Invalid option symbol: {}", option_symbol),
            });
            continue;
        }

        // Validate action
        let action_raw = get_field(
            &record,
            &headers,
            &["操作", "买/卖", "买卖", "action", "Action", "Type"],
        )
        .unwrap_or_default();
        let action = normalize_action(&action_raw);
        if action.is_empty() {
            error_rows.push(ImportError {
                row: i + 2,
                column: "操作".to_string(),
                message: format!("Invalid action: {}", action_raw),
            });
            continue;
        }

        // Build preview row
        let mut row_map = serde_json::Map::new();
        for (col_idx, header) in headers.iter().enumerate() {
            let val = record.get(col_idx).unwrap_or("").trim().to_string();
            row_map.insert(header.trim().to_string(), serde_json::Value::String(val));
        }
        preview_data.push(serde_json::Value::Object(row_map));
        valid_rows += 1;
    }

    Ok(ImportPreview {
        total_rows,
        valid_rows,
        error_rows,
        preview_data,
        column_mapping,
    })
}

// --- Helper types and functions ---

#[derive(Debug, serde::Deserialize)]
pub struct StockPriceInput {
    pub symbol: String,
    pub price: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct ImportOptionsResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Normalize action value to "SELL" or "BUY", supporting Chinese and English variants
/// ("卖"/"卖出", "SELL", "SELL TO OPEN", "Buy to Close", ...)
fn normalize_action(raw: &str) -> String {
    let s = raw.trim().to_uppercase();
    // English: "BUY", "BUY TO OPEN", "BUY TO CLOSE", ... and Chinese variants
    if s.starts_with("BUY") || raw.trim().contains('买') {
        "BUY".to_string()
    } else if s.starts_with("SELL") || raw.trim().contains('卖') {
        "SELL".to_string()
    } else {
        String::new()
    }
}

/// Parse an optional decimal CSV field. Blank fields retain the importer's
/// historical default of zero; malformed and non-finite values are errors.
fn parse_decimal(s: &str, field: &str) -> Result<f64, String> {
    let normalized = s.trim().replace(',', "");
    if normalized.is_empty() {
        return Ok(0.0);
    }

    let value = normalized
        .parse::<f64>()
        .map_err(|_| format!("invalid {} '{}'", field, s))?;
    if !value.is_finite() {
        return Err(format!("invalid {} '{}'", field, s));
    }
    Ok(value)
}

fn parse_required_decimal(s: &str, field: &str) -> Result<f64, String> {
    if s.trim().is_empty() {
        return Err(format!("invalid {} '{}'", field, s));
    }
    parse_decimal(s, field)
}

/// Parse quantity as a whole number of contracts without silently truncating.
fn parse_quantity(s: &str) -> Result<i64, String> {
    const I64_MIN_AS_F64: f64 = -9_223_372_036_854_775_808.0;
    const I64_MAX_EXCLUSIVE_AS_F64: f64 = 9_223_372_036_854_775_808.0;

    let value = parse_required_decimal(s, "quantity")?;
    if value == 0.0
        || value.fract() != 0.0
        || !(I64_MIN_AS_F64..I64_MAX_EXCLUSIVE_AS_F64).contains(&value)
    {
        return Err(format!("invalid quantity '{}'", s));
    }
    Ok(value as i64)
}

/// Whether a code on a BUY record terminates an open (SELL + O*) position.
/// `C` is a plain buy-to-close; `C;Ep` expired worthless, `A;C` assigned and
/// closed, `C;P` closed via exercise/put.
fn is_close_code(code: &str) -> bool {
    code == "C" || code == "C;Ep" || code == "A;C" || code == "C;P"
}

/// Parse an option expiry like "16JAN26" into (year, month, day).
fn parse_expiry_ymd(e: &str) -> Option<(i32, u32, u32)> {
    let months: std::collections::HashMap<&str, u32> = [
        ("JAN", 1),
        ("FEB", 2),
        ("MAR", 3),
        ("APR", 4),
        ("MAY", 5),
        ("JUN", 6),
        ("JUL", 7),
        ("AUG", 8),
        ("SEP", 9),
        ("OCT", 10),
        ("NOV", 11),
        ("DEC", 12),
    ]
    .iter()
    .cloned()
    .collect();
    if e.len() >= 7 {
        let day: u32 = e[0..2].parse().ok()?;
        let mon: u32 = *months.get(&e[2..5].to_uppercase().as_str())?;
        let yr: i32 = 2000 + e[5..7].parse::<i32>().ok()?;
        Some((yr, mon, day))
    } else {
        None
    }
}

/// Parse a split date like "2023-01-01" into (year, month, day).
fn parse_split_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 3 {
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ))
    } else {
        None
    }
}

/// Convert expiry date like "16JAN26" to sortable "2026-01-16" format.
fn parse_expiry_to_sortable(expiry: &str) -> String {
    let expiry = expiry.trim();
    if expiry.len() < 7 {
        return expiry.to_string();
    }
    let day = &expiry[0..2];
    let mon_str = &expiry[2..5];
    let year_short = &expiry[5..];

    let month = match mon_str.to_uppercase().as_str() {
        "JAN" => "01",
        "FEB" => "02",
        "MAR" => "03",
        "APR" => "04",
        "MAY" => "05",
        "JUN" => "06",
        "JUL" => "07",
        "AUG" => "08",
        "SEP" => "09",
        "OCT" => "10",
        "NOV" => "11",
        "DEC" => "12",
        _ => return expiry.to_string(),
    };

    let year = if let Ok(y) = year_short.parse::<u32>() {
        2000 + y
    } else {
        return expiry.to_string();
    };

    format!("{}-{}-{}", year, month, day)
}

/// Helper to get field by trying multiple header names (case-insensitive)
fn get_field(
    record: &csv::StringRecord,
    headers: &csv::StringRecord,
    names: &[&str],
) -> Option<String> {
    for name in names {
        let expected = name.to_lowercase();
        if let Some(idx) = headers
            .iter()
            .position(|h| h.trim().to_lowercase() == expected)
        {
            if let Some(val) = record.get(idx) {
                let trimmed = val.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }
    }
    None
}

/// Internal helper that doesn't require State wrapper.
/// Recomputes contract_status before loading so persisted records immediately
/// pick up the current FIFO matching rules.
pub fn get_option_contracts_inner(
    db: &Database,
    account_id: &str,
) -> Result<Vec<OptionContract>, String> {
    recompute_option_statuses(db, account_id)?;

    // Fetch all records — open records have pre-computed contract_status;
    // close records are only needed to display close_price/close_code.
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, option_symbol, underlying, expiry_date, strike_price,
                    option_type, action, code, quantity, price, amount, commission, fee,
                    traded_at, settled_at, created_at, contract_status
             FROM option_records WHERE account_id = ?1
             ORDER BY option_symbol, traded_at",
        )
        .map_err(|e| e.to_string())?;

    let records: Vec<OptionRecord> = stmt
        .query_map(rusqlite::params![account_id], |row| {
            Ok(OptionRecord {
                id: row.get(0)?,
                account_id: row.get(1)?,
                option_symbol: row.get(2)?,
                underlying: row.get(3)?,
                expiry_date: row.get(4)?,
                strike_price: row.get(5)?,
                option_type: row.get(6)?,
                action: row.get(7)?,
                code: row.get(8)?,
                quantity: row.get(9)?,
                price: row.get(10)?,
                amount: row.get(11)?,
                commission: row.get(12)?,
                fee: row.get(13)?,
                traded_at: row.get(14)?,
                settled_at: row.get(15)?,
                created_at: row.get(16)?,
                contract_status: row.get(17)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // Group by option_symbol
    let mut grouped: std::collections::HashMap<String, Vec<OptionRecord>> =
        std::collections::HashMap::new();
    for record in records {
        grouped
            .entry(record.option_symbol.clone())
            .or_default()
            .push(record);
    }

    let mut contracts: Vec<OptionContract> = Vec::new();

    for recs in grouped.values() {
        // Open records (SELL + code starts with "O")
        let mut opens: Vec<&OptionRecord> = recs
            .iter()
            .filter(|r| r.action == "SELL" && r.code.starts_with("O"))
            .collect();
        opens.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));

        if opens.is_empty() {
            continue;
        }

        // Close records
        let mut closes: Vec<&OptionRecord> = recs
            .iter()
            .filter(|r| r.action == "BUY" && is_close_code(&r.code))
            .collect();
        closes.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));

        let mut remaining_by_open: Vec<_> = opens
            .iter()
            .map(|open| (open.id.as_str(), open.quantity.abs()))
            .collect();
        let mut completing_close_by_open = std::collections::HashMap::new();
        for close in closes {
            let mut remaining_close = close.quantity.abs();
            for (open_id, remaining_open) in &mut remaining_by_open {
                if remaining_close == 0 {
                    break;
                }
                if *remaining_open == 0 {
                    continue;
                }

                let matched = (*remaining_open).min(remaining_close);
                *remaining_open -= matched;
                remaining_close -= matched;
                if *remaining_open == 0 {
                    completing_close_by_open.insert(*open_id, close);
                }
            }
        }

        for open in &opens {
            let status = open.contract_status.clone();
            let is_expired = status != "active";
            let completing_close = completing_close_by_open.get(open.id.as_str());
            contracts.push(OptionContract {
                id: open.id.clone(),
                option_symbol: open.option_symbol.clone(),
                underlying: open.underlying.clone(),
                expiry_date: open.expiry_date.clone(),
                strike_price: open.strike_price,
                option_type: open.option_type.clone(),
                contracts: open.quantity,
                open_price: open.price,
                open_amount: open.amount,
                commission: open.commission,
                traded_at: open.traded_at.clone(),
                close_price: if is_expired {
                    completing_close.map(|record| record.price)
                } else {
                    None
                },
                close_code: if is_expired {
                    completing_close.map(|record| record.code.clone())
                } else {
                    None
                },
                status: status.clone(),
                account_id: open.account_id.clone(),
            });
        }
    }

    contracts.sort_by(|a, b| {
        a.underlying
            .cmp(&b.underlying)
            .then_with(|| {
                parse_expiry_to_sortable(&a.expiry_date)
                    .cmp(&parse_expiry_to_sortable(&b.expiry_date))
            })
            .then_with(|| {
                a.strike_price
                    .partial_cmp(&b.strike_price)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    Ok(contracts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    /// Build an in-memory DB with one US account.
    fn db_with_account() -> (Database, String) {
        let db = Database::new(":memory:").expect("failed to create in-memory database");
        let account_id = "acct-test".to_string();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES (?1, ?2, 'US', ?3, ?3)",
                rusqlite::params![account_id, "Test Account", chrono::Utc::now().to_rfc3339()],
            )
            .expect("failed to insert account");
        }
        (db, account_id)
    }

    /// A sample IBKR-style English-header options trade CSV.
    /// All close records have a matching open record (same symbol, enough quantity).
    const ENGLISH_CSV: &str = "\
Acct ID,Symbol,Trade Date/Time,Settle Date,Exchange,Type,Quantity,Price,Proceeds,Comm,Fee,Order Type,Code
U1234567,AAPL 20FEB26 100 P,2026-01-15 10:30:00,2026-01-16,SMART,SELL,2,3.50,700,1.20,0.05,LMT,O
U1234567,AAPL 20FEB26 100 P,2026-01-15 10:30:00,2026-01-16,SMART,SELL,1,3.50,350,1.20,0.05,LMT,O
U1234567,PDD 20MAR26 80 C,2026-01-10 09:30:00,2026-01-11,SMART,SELL,3,1.50,450,1.00,0.04,LMT,O
U1234567,PDD 20MAR26 80 C,2026-02-20 09:45:00,2026-02-21,SMART,BUY TO CLOSE,3,2.00,600,0.90,0.04,MKT,C;P
Total, ,,,,,,,,,,,
";

    #[test]
    fn test_import_english_header_csv() {
        let (db, account_id) = db_with_account();
        let result =
            import_options_csv_inner(&db, &account_id, ENGLISH_CSV).expect("import should succeed");
        assert_eq!(result.imported, 4, "all 4 trade rows should import");
        assert_eq!(result.skipped, 1, "the Total row should be skipped");
        assert!(
            result.errors.is_empty(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_import_rejects_malformed_price() {
        let (db, account_id) = db_with_account();
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,oops,200.00,0,0,LMT,O
";

        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

        assert_eq!(result.imported, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("Row 2"));
        assert!(result.errors[0].contains("price"));
        let count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_import_rejects_blank_price() {
        let (db, account_id) = db_with_account();
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,,200.00,0,0,LMT,O
";

        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

        assert_eq!(result.imported, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("price"));
    }

    #[test]
    fn test_import_rejects_non_finite_amount() {
        let (db, account_id) = db_with_account();
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,NaN,0,0,LMT,O
";

        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

        assert_eq!(result.imported, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("Row 2"));
        assert!(result.errors[0].contains("amount"));
    }

    #[test]
    fn test_import_rejects_malformed_quantity() {
        let (db, account_id) = db_with_account();
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1.5,2.00,200.00,0,0,LMT,O
";

        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("Row 2"));
        assert!(result.errors[0].contains("quantity"));
    }

    #[test]
    fn test_import_rejects_zero_quantity() {
        let (db, account_id) = db_with_account();
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,0,2.00,200.00,0,0,LMT,O
";

        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("quantity"));
    }

    #[test]
    fn test_import_rolls_back_all_rows_when_insert_fails() {
        let (db, account_id) = db_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER reject_second_option
                 BEFORE INSERT ON option_records
                 WHEN NEW.option_symbol = 'MSFT 20FEB26 100 P'
                 BEGIN
                   SELECT RAISE(ABORT, 'forced option insert failure');
                 END;",
            )
            .unwrap();
        }
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
a,MSFT 20FEB26 100 P,2026-01-15,,SMART,卖出,1,3.00,300.00,0,0,LMT,O
";

        let error = import_options_csv_inner(&db, &account_id, csv)
            .expect_err("database failure should abort the import");

        assert!(error.contains("forced option insert failure"));
        let count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "the first insert must be rolled back too");
    }

    #[test]
    fn test_import_rolls_back_rows_when_status_recompute_fails() {
        let (db, account_id) = db_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER reject_option_status_update
                 BEFORE UPDATE OF contract_status ON option_records
                 BEGIN
                   SELECT RAISE(ABORT, 'forced option status failure');
                 END;",
            )
            .unwrap();
        }
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
";

        let error = import_options_csv_inner(&db, &account_id, csv)
            .expect_err("status failure should abort the import");

        assert!(error.contains("forced option status failure"));
        let count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "insert must roll back with status updates");
    }

    #[test]
    fn test_parse_english_header_csv_preview() {
        let preview = parse_options_csv(ENGLISH_CSV.to_string()).expect("preview should succeed");
        assert_eq!(preview.valid_rows, 4);
        assert!(
            preview.error_rows.is_empty(),
            "expected no errors, got: {:?}",
            preview.error_rows
        );
    }

    /// A Chinese-header options trade CSV with one open and one close record.
    const CN_CSV: &str = "\
账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
a,AAPL 20FEB26 100 P,2026-02-20,,SMART,买入,1,0.10,10.00,0,0,C;P,C;P
";

    #[test]
    fn test_import_rejects_close_without_open() {
        // A close record (C;Ep expired) with no open record anywhere must be rejected.
        let (db, account_id) = db_with_account();
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-02-20,,SMART,买入,1,0.01,1.00,0,0,C;Ep,C;Ep
";
        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
        assert_eq!(result.imported, 0, "orphan close must not be inserted");
        assert_eq!(
            result.errors.len(),
            1,
            "expected one error, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_import_close_matches_open_in_same_csv() {
        let (db, account_id) = db_with_account();
        let result =
            import_options_csv_inner(&db, &account_id, CN_CSV).expect("import should succeed");
        assert_eq!(result.imported, 2, "open + close should both import");
        assert!(
            result.errors.is_empty(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_import_close_matches_existing_db_open() {
        // Open already in DB (from a previous import); only the close is imported now.
        let (db, account_id) = db_with_account();
        let ts = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                 VALUES ('o1', ?1, 'AAPL 20FEB26 100 P', 'AAPL', '20FEB26', 100, 'P', 'SELL', 'O', 1, 2.00, 200.00, 0, 0, '2026-01-15', NULL, ?2, 'active')",
                rusqlite::params![account_id, ts],
            )
            .expect("failed to insert open record");
        }
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-02-20,,SMART,买入,1,0.10,10.00,0,0,C;P,C;P
";
        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
        assert_eq!(result.imported, 1, "close should match the existing open");
        assert!(
            result.errors.is_empty(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_import_close_exceeding_open_quantity_rejected() {
        // Open 1 contract but close 2 contracts: the extra close has no backing open.
        let (db, account_id) = db_with_account();
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
a,AAPL 20FEB26 100 P,2026-02-20,,SMART,买入,2,0.10,20.00,0,0,C;P,C;P
";
        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
        assert_eq!(result.imported, 1, "only the open should import");
        assert_eq!(
            result.errors.len(),
            1,
            "close exceeding open qty must be rejected"
        );
    }

    #[test]
    fn test_import_split_adjusted_close_matches() {
        // Contract split 2:1 configured in settings: open at strike 330 (BRK B),
        // close at strike 165 (post-split symbol). Must match cross-symbol.
        let (db, account_id) = db_with_account();
        let ts = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO stock_splits (stock_code, split_date, ratio_from, ratio_to, created_at)
                 VALUES ('BRK B', '2023-01-01', 1, 2, ?1)",
                rusqlite::params![ts],
            )
            .expect("failed to insert stock split");
        }
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,BRK B 16JUN23 330 C,2023-01-10,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
a,BRK B 16JUN23 165 C,2023-06-10,,SMART,买入,1,0.10,10.00,0,0,C;P,C;P
";
        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
        assert_eq!(
            result.imported, 2,
            "split-adjusted close should match via split config"
        );
        assert!(
            result.errors.is_empty(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_normalize_action_english_variants() {
        assert_eq!(normalize_action("SELL"), "SELL");
        assert_eq!(normalize_action("SELL TO OPEN"), "SELL");
        assert_eq!(normalize_action("Buy to Close"), "BUY");
        assert_eq!(normalize_action("buy"), "BUY");
        assert_eq!(normalize_action("卖出开仓"), "SELL");
        assert_eq!(normalize_action("买入平仓"), "BUY");
        assert_eq!(normalize_action("unknown"), "");
    }

    /// An account whose records are all 'active' (e.g. only open positions,
    /// or every close was rejected by the import boundary check) must not
    /// cause get_option_contracts_inner to recompute endlessly and overflow
    /// the stack — it should return the contracts normally.
    #[test]
    fn test_get_contracts_all_active_no_stack_overflow() {
        let (db, account_id) = db_with_account();
        let ts = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            // Open positions only — no close records, all contract_status = 'active'
            for (id, symbol, strike) in [
                ("o1", "AAPL 20FEB26 100 P", 100.0),
                ("o2", "TSLA 20MAR26 250 C", 250.0),
            ] {
                conn.execute(
                    "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'SELL', 'O', 1, 2.00, 200.00, 0, 0, '2026-01-15', NULL, ?8, 'active')",
                    rusqlite::params![id, account_id, symbol, symbol.split(' ').next().unwrap(), "20FEB26", strike, "P", ts],
                )
                .unwrap();
            }
        }
        let contracts = get_option_contracts_inner(&db, &account_id).expect("should not crash");
        assert_eq!(contracts.len(), 2, "both open contracts should be returned");
    }

    #[test]
    fn test_get_field_case_insensitive() {
        let headers = csv::StringRecord::from(vec![
            "SYMBOL".to_string(),
            "Quantity".to_string(),
            "Trade Date/Time".to_string(),
        ]);
        let record = csv::StringRecord::from(vec![
            "AAPL 20FEB26 100 P".to_string(),
            "2".to_string(),
            "2026-01-15 10:30:00".to_string(),
        ]);
        assert_eq!(
            get_field(&record, &headers, &["Symbol"]).as_deref(),
            Some("AAPL 20FEB26 100 P")
        );
        assert_eq!(
            get_field(&record, &headers, &["quantity"]).as_deref(),
            Some("2")
        );
        assert_eq!(
            get_field(&record, &headers, &["trade date/time"]).as_deref(),
            Some("2026-01-15 10:30:00")
        );
    }

    /// User-reported scenario: sell call 200 contracts (SELL O, qty -200),
    /// buy back 100 (BUY code C), 100 expire (BUY code C;Ep). Total close qty
    /// 200 matches open qty 200, so the open must NOT stay 'active'.
    fn insert_857_scenario(conn: &rusqlite::Connection, account_id: &str, ts: &str) {
        for (id, action, code, qty, traded) in [
            ("r1", "SELL", "O", -200, "2023-09-06, 22:47:47"),
            ("r2", "BUY", "C", 100, "2023-09-13, 01:08:34"),
            ("r3", "BUY", "C;Ep", 100, "2023/10/30"),
        ] {
            conn.execute(
                "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                 VALUES (?1, ?2, '857 30OCT23 6 C', '857', '30OCT23', 6.0, 'C', ?3, ?4, ?5, 0.1, 0.0, 0.0, 0.0, ?6, NULL, ?7, 'active')",
                rusqlite::params![id, account_id, action, code, qty, traded, ts],
            )
            .unwrap();
        }
    }

    #[test]
    fn test_recompute_plain_c_close_matches_and_expires() {
        let (db, account_id) = db_with_account();
        let ts = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            insert_857_scenario(&conn, &account_id, &ts);
        }
        recompute_option_statuses(&db, &account_id).expect("recompute should succeed");
        let conn = db.conn.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT contract_status FROM option_records WHERE id = 'r1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "expired",
            "sell 200 with 100 closed (C) + 100 expired (C;Ep) should be expired, got {}",
            status
        );
    }

    #[test]
    fn test_recompute_plain_c_close_only_marks_closed() {
        // SELL O 100 then BUY C 100 → fully closed via plain C code.
        let (db, account_id) = db_with_account();
        let ts = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            for (id, action, code, qty, traded) in [
                ("r1", "SELL", "O", -100, "2023-09-06, 22:47:47"),
                ("r2", "BUY", "C", 100, "2023-09-13, 01:08:34"),
            ] {
                conn.execute(
                    "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                     VALUES (?1, ?2, '857 30OCT23 6 C', '857', '30OCT23', 6.0, 'C', ?3, ?4, ?5, 0.1, 0.0, 0.0, 0.0, ?6, NULL, ?7, 'active')",
                    rusqlite::params![id, account_id, action, code, qty, traded, ts],
                )
                .unwrap();
            }
        }
        recompute_option_statuses(&db, &account_id).expect("recompute should succeed");
        let conn = db.conn.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT contract_status FROM option_records WHERE id = 'r1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "closed",
            "sell 100 then buy back 100 (C) should be closed, got {}",
            status
        );
    }

    #[test]
    fn test_partial_group_close_completes_oldest_open_fifo() {
        // Two opens share one option symbol. Closing 10 of 150 contracts must
        // complete the oldest 10-contract open while leaving the other 140 active.
        let (db, account_id) = db_with_account();
        let ts = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            for (id, action, code, qty, traded) in [
                ("o1", "SELL", "O", -10, "2024-10-01, 10:01:41"),
                ("o2", "SELL", "O", -140, "2024-10-01, 11:30:40"),
                ("c1", "BUY", "C", 10, "2024-12-04, 09:57:34"),
            ] {
                conn.execute(
                    "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                     VALUES (?1, ?2, 'BABA 15JAN27 160 C', 'BABA', '15JAN27', 160.0, 'C', ?3, ?4, ?5, 1.0, 0.0, 0.0, 0.0, ?6, NULL, ?7, 'active')",
                    rusqlite::params![id, account_id, action, code, qty, traded, ts],
                )
                .unwrap();
            }
        }

        let contracts = get_option_contracts_inner(&db, &account_id)
            .expect("partial close should produce option contracts");
        let oldest = contracts
            .iter()
            .find(|contract| contract.id == "o1")
            .expect("oldest open should be returned");
        let remaining = contracts
            .iter()
            .find(|contract| contract.id == "o2")
            .expect("remaining open should be returned");

        assert_eq!(oldest.status, "closed");
        assert_eq!(oldest.close_code.as_deref(), Some("C"));
        assert_eq!(remaining.status, "active");
        assert_eq!(remaining.close_code, None);
        assert_eq!(
            contracts
                .iter()
                .filter(|contract| contract.status != "active")
                .map(|contract| contract.contracts.abs())
                .sum::<i64>(),
            10,
        );
        assert_eq!(
            contracts
                .iter()
                .filter(|contract| contract.status == "active")
                .map(|contract| contract.contracts.abs())
                .sum::<i64>(),
            140,
        );
    }

    #[test]
    fn test_fifo_contracts_keep_each_opens_completing_close_details() {
        let (db, account_id) = db_with_account();
        let ts = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            for (id, action, code, qty, price, traded) in [
                ("o1", "SELL", "O", -10, 3.0, "2024-10-01, 10:01:41"),
                ("o2", "SELL", "O", -10, 4.0, "2024-10-01, 11:30:40"),
                ("c1", "BUY", "C", 10, 2.0, "2024-12-04, 09:57:34"),
                ("c2", "BUY", "C;Ep", 10, 0.0, "2025-01-15"),
            ] {
                conn.execute(
                    "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                     VALUES (?1, ?2, 'BABA 15JAN27 160 C', 'BABA', '15JAN27', 160.0, 'C', ?3, ?4, ?5, ?6, 0.0, 0.0, 0.0, ?7, NULL, ?8, 'active')",
                    rusqlite::params![id, account_id, action, code, qty, price, traded, ts],
                )
                .unwrap();
            }
        }

        let contracts = get_option_contracts_inner(&db, &account_id)
            .expect("completed FIFO opens should be returned");
        let oldest = contracts
            .iter()
            .find(|contract| contract.id == "o1")
            .expect("oldest open should be returned");
        let newest = contracts
            .iter()
            .find(|contract| contract.id == "o2")
            .expect("newest open should be returned");

        assert_eq!(oldest.status, "closed");
        assert_eq!(oldest.close_code.as_deref(), Some("C"));
        assert_eq!(oldest.close_price, Some(2.0));
        assert_eq!(newest.status, "expired");
        assert_eq!(newest.close_code.as_deref(), Some("C;Ep"));
        assert_eq!(newest.close_price, Some(0.0));
    }

    #[test]
    fn test_export_round_trip_plain_c_close_matches() {
        // User-reported: export → clear → re-import must preserve matching.
        // SELL O 200, BUY C 100, BUY C;Ep 100 → open must be 'expired'.
        let (db, account_id) = db_with_account();
        let ts = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            insert_857_scenario(&conn, &account_id, &ts);
        }
        let csv = export_options_csv_inner(&db, &account_id).expect("export should succeed");
        // Clear all records, then re-import the exported CSV.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM option_records WHERE account_id = ?1",
                rusqlite::params![account_id],
            )
            .unwrap();
        }
        let result =
            import_options_csv_inner(&db, &account_id, &csv).expect("import should succeed");
        assert_eq!(
            result.imported, 3,
            "all 3 rows re-imported, got {:?}",
            result.errors
        );
        // After import, recompute should mark the open (SELL O) as expired.
        recompute_option_statuses(&db, &account_id).expect("recompute should succeed");
        let conn = db.conn.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT contract_status FROM option_records
                 WHERE account_id = ?1 AND action = 'SELL' AND option_symbol = '857 30OCT23 6 C'",
                rusqlite::params![account_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "expired",
            "round-trip open should be expired, got {}",
            status
        );
    }

    #[test]
    fn test_import_plain_c_close_accepted_with_open() {
        // Import boundary check must treat plain code C as a close record.
        let (db, account_id) = db_with_account();
        let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,857 30OCT23 6 C,2023-09-06,,SMART,卖出,200,0.13,52000.00,-204,0,LMT,O
a,857 30OCT23 6 C,2023-09-13,,SMART,买入,100,0.07,14000.00,-78,0,C,C
a,857 30OCT23 6 C,2023-10-30,,SMART,买入,100,0.00,0.00,0,0,C;Ep,C;Ep
";
        let result =
            import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
        assert_eq!(
            result.imported, 3,
            "open + C close + C;Ep close should all import"
        );
        assert!(
            result.errors.is_empty(),
            "expected no errors, got: {:?}",
            result.errors
        );
    }
}
