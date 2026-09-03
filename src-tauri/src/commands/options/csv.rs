use super::contracts;
use crate::db::Database;
use crate::services::option_matching::{match_options_fifo, MatchRecord};
use tracing::warn;

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

pub(super) fn import_options_csv_inner(
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
    let (existing_records, splits) = contracts::load_matching_inputs(&transaction, account_id)?;
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
        contracts::recompute_option_statuses_in(&transaction, account_id)?;
    }
    transaction.commit().map_err(|e| e.to_string())?;

    Ok(ImportOptionsResult {
        imported,
        skipped,
        errors,
    })
}

/// Export all option records for an account as a CSV string.
pub(super) fn export_options_csv_inner(db: &Database, account_id: &str) -> Result<String, String> {
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
pub(super) fn parse_options_csv_inner(
    csv_content: &str,
) -> Result<crate::models::import_export::ImportPreview, String> {
    use crate::models::import_export::{ImportError, ImportPreview};
    use std::collections::HashMap;

    let content = csv_content.strip_prefix('\u{feff}').unwrap_or(csv_content);

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

#[derive(Debug, serde::Serialize)]
pub struct ImportOptionsResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Normalize action value to "SELL" or "BUY", supporting Chinese and English variants
/// ("卖"/"卖出", "SELL", "SELL TO OPEN", "Buy to Close", ...)
pub(super) fn normalize_action(raw: &str) -> String {
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
pub(super) fn parse_expiry_to_sortable(expiry: &str) -> String {
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
pub(super) fn get_field(
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
