use crate::db::Database;
use crate::models::option::{
    CallContractSimulation, OptionContract, OptionRecord, PutContractSimulation,
    SellCallSimulation, SellPutSimulation,
};
use crate::services::option_matching::{match_options_fifo, MatchRecord, SplitRecord};
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
    id: String,
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

fn parsed_row_match_record(row: &ParsedOptionRow) -> MatchRecord {
    MatchRecord {
        id: row.id.clone(),
        option_symbol: row.option_symbol.clone(),
        underlying: row.underlying.clone(),
        expiry_date: row.expiry_date.clone(),
        strike_price: row.strike_price,
        option_type: row.option_type.clone(),
        action: row.action.clone(),
        code: row.code.clone(),
        quantity: row.quantity,
        traded_at: row.traded_at.clone(),
    }
}

fn load_matching_inputs(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<(Vec<MatchRecord>, Vec<SplitRecord>), String> {
    let records = {
        let mut statement = conn
            .prepare(
                "SELECT id, option_symbol, underlying, expiry_date, strike_price,
                        option_type, action, code, quantity, traded_at
                 FROM option_records WHERE account_id = ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([account_id], |row| {
                Ok(MatchRecord {
                    id: row.get(0)?,
                    option_symbol: row.get(1)?,
                    underlying: row.get(2)?,
                    expiry_date: row.get(3)?,
                    strike_price: row.get(4)?,
                    option_type: row.get(5)?,
                    action: row.get(6)?,
                    code: row.get(7)?,
                    quantity: row.get(8)?,
                    traded_at: row.get(9)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let splits = load_split_records(conn)?;
    Ok((records, splits))
}

fn load_split_records(conn: &rusqlite::Connection) -> Result<Vec<SplitRecord>, String> {
    let mut statement = conn
        .prepare("SELECT stock_code, split_date, ratio_from, ratio_to FROM stock_splits")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(SplitRecord {
                stock_code: row.get(0)?,
                split_date: row.get(1)?,
                ratio_from: row.get(2)?,
                ratio_to: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
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
            id: uuid::Uuid::new_v4().to_string(),
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

    // ---- Boundary check: use the same conserved FIFO engine as status and review ----
    let (existing_records, splits) = load_matching_inputs(&transaction, account_id)?;
    let mut accepted: Vec<&ParsedOptionRow> = parsed.iter().collect();
    loop {
        let mut candidates = existing_records.clone();
        candidates.extend(accepted.iter().map(|row| parsed_row_match_record(row)));
        let result = match_options_fifo(&candidates, &splits);
        let rejected_ids: std::collections::HashSet<_> = accepted
            .iter()
            .filter(|row| row.action == "BUY" && is_close_code(&row.code))
            .filter(|row| result.unmatched_close_ids.contains(&row.id))
            .map(|row| row.id.as_str())
            .collect();
        if rejected_ids.is_empty() {
            break;
        }
        accepted.retain(|row| !rejected_ids.contains(row.id.as_str()));
    }
    let accepted_ids: std::collections::HashSet<_> =
        accepted.iter().map(|row| row.id.as_str()).collect();
    for row in parsed
        .iter()
        .filter(|row| !accepted_ids.contains(row.id.as_str()))
    {
        errors.push(format!(
            "Row {}: close record {} ({}) has no matching open record; skipped. \
             If the contract was split-adjusted, configure the split info in Settings first",
            row.row_num, row.option_symbol, row.code
        ));
    }

    // ---- Pass 2: insert the accepted subset ----
    for row in accepted {
        let now = chrono::Utc::now().to_rfc3339();

        transaction.execute(
            "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, 'active')",
            rusqlite::params![
                row.id,
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
#[cfg(test)]
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
    let (records, splits) = load_matching_inputs(conn, account_id)?;
    let result = match_options_fifo(&records, &splits);
    let close_codes: std::collections::HashMap<_, _> = records
        .iter()
        .map(|record| (record.id.as_str(), record.code.as_str()))
        .collect();

    conn.execute(
        "UPDATE option_records SET contract_status = 'active' WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| e.to_string())?;

    for open in records
        .iter()
        .filter(|record| record.action == "SELL" && record.code.starts_with('O'))
    {
        if result.remaining_open.get(&open.id).copied().unwrap_or(0) != 0 {
            continue;
        }
        let Some(completing_close) = result
            .allocations
            .iter()
            .rev()
            .find(|allocation| allocation.open_id == open.id)
        else {
            continue;
        };
        let status = match close_codes.get(completing_close.close_id.as_str()).copied() {
            Some("A;C") => "assigned",
            Some("C") | Some("C;P") => "closed",
            _ => "expired",
        };
        conn.execute(
            "UPDATE option_records SET contract_status = ?1 WHERE id = ?2",
            rusqlite::params![status, open.id],
        )
        .map_err(|e| e.to_string())?;
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
/// Returned status is projected from the shared matcher without mutating rows.
pub fn get_option_contracts_inner(
    db: &Database,
    account_id: &str,
) -> Result<Vec<OptionContract>, String> {
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
    let match_records: Vec<_> = records
        .iter()
        .map(|record| MatchRecord {
            id: record.id.clone(),
            option_symbol: record.option_symbol.clone(),
            underlying: record.underlying.clone(),
            expiry_date: record.expiry_date.clone(),
            strike_price: record.strike_price,
            option_type: record.option_type.clone(),
            action: record.action.clone(),
            code: record.code.clone(),
            quantity: record.quantity,
            traded_at: record.traded_at.clone(),
        })
        .collect();
    let result = match_options_fifo(&match_records, &load_split_records(&conn)?);
    let records_by_id: std::collections::HashMap<_, _> = records
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect();
    let mut contracts: Vec<OptionContract> = records
        .iter()
        .filter(|record| record.action == "SELL" && record.code.starts_with('O'))
        .map(|open| {
            let is_complete = result.remaining_open.get(&open.id).copied().unwrap_or(0) == 0;
            let completing_close = is_complete
                .then(|| {
                    result
                        .allocations
                        .iter()
                        .rev()
                        .find(|allocation| allocation.open_id == open.id)
                        .and_then(|allocation| records_by_id.get(allocation.close_id.as_str()))
                })
                .flatten();
            let status = completing_close
                .map(|close| match close.code.as_str() {
                    "A;C" => "assigned",
                    "C" | "C;P" => "closed",
                    _ => "expired",
                })
                .unwrap_or("active")
                .to_string();
            OptionContract {
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
                close_price: completing_close.map(|record| record.price),
                close_code: completing_close.map(|record| record.code.clone()),
                status,
                account_id: open.account_id.clone(),
            }
        })
        .collect();

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
                 VALUES ('BRK B', '2023-02-01', 1, 2, ?1)",
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
    fn test_get_contracts_projects_status_without_rewriting_rows() {
        let (db, account_id) = db_with_account();
        let created_at = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            for (id, action, code, quantity, price, traded_at) in [
                ("open", "SELL", "O", -1, 2.0, "2026-01-10"),
                ("close", "BUY", "C", 1, 0.5, "2026-01-20"),
            ] {
                conn.execute(
                    "INSERT INTO option_records
                     (id, account_id, option_symbol, underlying, expiry_date, strike_price,
                      option_type, action, code, quantity, price, amount, commission, fee,
                      traded_at, created_at, contract_status)
                     VALUES (?1, ?2, 'ACME 20FEB26 100 P', 'ACME', '20FEB26', 100,
                             'P', ?3, ?4, ?5, ?6, 0, 0, 0, ?7, ?8, 'active')",
                    rusqlite::params![
                        id, account_id, action, code, quantity, price, traded_at, created_at
                    ],
                )
                .unwrap();
            }
        }

        let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
        assert_eq!(contracts[0].status, "closed");
        assert_eq!(contracts[0].close_code.as_deref(), Some("C"));

        let persisted: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT contract_status FROM option_records WHERE id = 'open'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted, "active");
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
