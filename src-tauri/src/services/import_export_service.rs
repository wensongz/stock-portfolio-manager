use crate::db::Database;
use crate::models::import_export::{
    ExportFilters, ImportData, ImportError, ImportPreview, ImportResult,
};
use chrono::Utc;
use csv::WriterBuilder;
use std::collections::HashMap;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Export
// ─────────────────────────────────────────────────────────────────────────────

/// Export holdings to CSV and return the CSV string content.
pub fn export_holdings_csv(db: &Database, filters: &ExportFilters) -> Result<String, String> {
    let conn = db.conn.lock().unwrap();

    // Build parameterized query
    let mut conditions = Vec::new();
    let mut params: Vec<String> = Vec::new();

    if let Some(market) = &filters.market {
        if !market.is_empty() {
            conditions.push(format!("h.market = ?{}", params.len() + 1));
            params.push(market.clone());
        }
    }
    if let Some(account_id) = &filters.account_id {
        if !account_id.is_empty() {
            conditions.push(format!("h.account_id = ?{}", params.len() + 1));
            params.push(account_id.clone());
        }
    }
    if let Some(cat_id) = &filters.category_id {
        if !cat_id.is_empty() {
            conditions.push(format!("h.category_id = ?{}", params.len() + 1));
            params.push(cat_id.clone());
        }
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let query = format!(
        "SELECT a.name as account_name, h.symbol, h.name, h.market,
                c.name as category_name, h.shares, h.avg_cost, h.currency
         FROM holdings h
         LEFT JOIN accounts a ON a.id = h.account_id
         LEFT JOIN categories c ON c.id = h.category_id
         {}
         ORDER BY h.market, h.symbol",
        where_clause
    );

    let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;

    let mut wtr = WriterBuilder::new().from_writer(vec![]);
    wtr.write_record(&[
        "账户名", "股票代码", "股票名称", "市场", "类别",
        "持仓数量", "均价", "币种",
    ])
    .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter()),
            |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, String>(2).unwrap_or_default(),
                    row.get::<_, String>(3).unwrap_or_default(),
                    row.get::<_, String>(4).unwrap_or_default(),
                    row.get::<_, f64>(5).unwrap_or(0.0),
                    row.get::<_, f64>(6).unwrap_or(0.0),
                    row.get::<_, String>(7).unwrap_or_default(),
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    for (account, symbol, name, market, category, shares, avg_cost, currency) in rows {
        wtr.write_record(&[
            account,
            symbol,
            name,
            market,
            category,
            shares.to_string(),
            avg_cost.to_string(),
            currency,
        ])
        .map_err(|e| e.to_string())?;
    }

    let data = wtr.into_inner().map_err(|e| e.to_string())?;
    String::from_utf8(data).map_err(|e| e.to_string())
}

/// Export transactions to CSV.
pub fn export_transactions_csv(
    db: &Database,
    start_date: &str,
    end_date: &str,
    filters: &ExportFilters,
) -> Result<String, String> {
    let conn = db.conn.lock().unwrap();

    let mut conditions = Vec::new();
    let mut params: Vec<String> = Vec::new();

    if !start_date.is_empty() {
        conditions.push(format!("t.traded_at >= ?{}", params.len() + 1));
        params.push(start_date.to_string());
    }
    if !end_date.is_empty() {
        conditions.push(format!("t.traded_at <= ?{}", params.len() + 1));
        params.push(end_date.to_string());
    }
    if let Some(market) = &filters.market {
        if !market.is_empty() {
            conditions.push(format!("t.market = ?{}", params.len() + 1));
            params.push(market.clone());
        }
    }
    if let Some(account_id) = &filters.account_id {
        if !account_id.is_empty() {
            conditions.push(format!("t.account_id = ?{}", params.len() + 1));
            params.push(account_id.clone());
        }
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let query = format!(
        "SELECT t.traded_at, a.name as account_name, t.symbol, t.name, t.market,
                t.transaction_type, t.shares, t.price, t.total_amount, t.commission,
                t.currency, t.notes
         FROM transactions t
         LEFT JOIN accounts a ON a.id = t.account_id
         {}
         ORDER BY t.traded_at DESC",
        where_clause
    );

    let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;

    let mut wtr = WriterBuilder::new().from_writer(vec![]);
    wtr.write_record(&[
        "交易日期", "账户名", "股票代码", "股票名称", "市场",
        "买卖方向", "数量", "价格", "金额", "手续费", "币种", "备注",
    ])
    .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter()),
            |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, String>(2).unwrap_or_default(),
                    row.get::<_, String>(3).unwrap_or_default(),
                    row.get::<_, String>(4).unwrap_or_default(),
                    row.get::<_, String>(5).unwrap_or_default(),
                    row.get::<_, f64>(6).unwrap_or(0.0),
                    row.get::<_, f64>(7).unwrap_or(0.0),
                    row.get::<_, f64>(8).unwrap_or(0.0),
                    row.get::<_, f64>(9).unwrap_or(0.0),
                    row.get::<_, String>(10).unwrap_or_default(),
                    row.get::<_, Option<String>>(11).unwrap_or(None).unwrap_or_default(),
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    for (traded_at, account, symbol, name, market, tx_type, shares, price, amount, comm, currency, notes) in rows {
        wtr.write_record(&[
            traded_at, account, symbol, name, market, tx_type,
            shares.to_string(), price.to_string(), amount.to_string(),
            comm.to_string(), currency, notes,
        ])
        .map_err(|e| e.to_string())?;
    }

    let data = wtr.into_inner().map_err(|e| e.to_string())?;
    String::from_utf8(data).map_err(|e| e.to_string())
}

/// Generate holdings import template CSV.
pub fn get_holdings_template() -> String {
    let mut wtr = WriterBuilder::new().from_writer(vec![]);
    let _ = wtr.write_record(&[
        "symbol", "name", "market", "shares", "avg_cost", "currency",
    ]);
    let _ = wtr.write_record(&["AAPL", "苹果", "US", "100", "150.00", "USD"]);
    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

/// Generate transactions import template CSV.
pub fn get_transactions_template() -> String {
    let mut wtr = WriterBuilder::new().from_writer(vec![]);
    let _ = wtr.write_record(&[
        "traded_at", "symbol", "name", "market", "transaction_type",
        "shares", "price", "commission", "currency", "notes",
    ]);
    let _ = wtr.write_record(&[
        "2024-01-15", "AAPL", "苹果", "US", "BUY",
        "100", "150.00", "0", "USD", "",
    ]);
    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// Import / Parse
// ─────────────────────────────────────────────────────────────────────────────

/// Parse CSV content and return an ImportPreview (validate but don't write).
pub fn parse_import_csv(content: &str, data_type: &str) -> Result<ImportPreview, String> {
    let mut rdr = csv::Reader::from_reader(content.as_bytes());

    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Required fields per data type
    let required_holdings = ["symbol", "shares", "avg_cost"];
    let required_transactions = ["traded_at", "symbol", "transaction_type", "shares", "price"];

    let required_fields: &[&str] = if data_type == "holdings" {
        &required_holdings
    } else {
        &required_transactions
    };

    let column_mapping: HashMap<String, String> = headers
        .iter()
        .map(|h| (h.clone(), h.clone()))
        .collect();

    let mut preview_data: Vec<serde_json::Value> = Vec::new();
    let mut error_rows: Vec<ImportError> = Vec::new();
    let mut total_rows = 0usize;

    for (i, result) in rdr.records().enumerate() {
        let record = result.map_err(|e| e.to_string())?;
        total_rows += 1;
        let row_num = i + 2; // 1-indexed, +1 for header

        let row_map: serde_json::Map<String, serde_json::Value> = headers
            .iter()
            .zip(record.iter())
            .map(|(h, v)| (h.clone(), serde_json::Value::String(v.to_string())))
            .collect();

        // Validate required fields
        let mut has_error = false;
        for field in required_fields {
            let val = row_map.get(*field).and_then(|v| v.as_str()).unwrap_or("");
            if val.trim().is_empty() {
                error_rows.push(ImportError {
                    row: row_num,
                    column: field.to_string(),
                    message: format!("第{}行 {} 字段不能为空", row_num, field),
                });
                has_error = true;
            }
        }

        // Validate market value against known enum
        if let Some(market_val) = row_map.get("market") {
            let market = market_val.as_str().unwrap_or("").trim();
            if !market.is_empty() && !["US", "CN", "HK"].contains(&market) {
                error_rows.push(ImportError {
                    row: row_num,
                    column: "market".to_string(),
                    message: format!("第{}行 market 必须为 US/CN/HK", row_num),
                });
                has_error = true;
            }
        }

        if !has_error && preview_data.len() < 20 {
            preview_data.push(serde_json::Value::Object(row_map));
        }
    }

    let valid_rows = total_rows.saturating_sub(error_rows.len());

    Ok(ImportPreview {
        total_rows,
        valid_rows,
        error_rows,
        preview_data,
        column_mapping,
    })
}

/// Extract a string field value from a JSON row.
fn extract_str<'a>(row: &'a serde_json::Value, key: &str) -> &'a str {
    row.get(key).and_then(|v| v.as_str()).unwrap_or("").trim()
}

/// Confirm and write import data to the database.
pub fn confirm_import(db: &Database, import_data: &ImportData) -> Result<ImportResult, String> {
    let conn = db.conn.lock().unwrap();
    let now = Utc::now().to_rfc3339();
    let mut imported_count = 0usize;
    let mut skipped_count = 0usize;
    let mut errors: Vec<ImportError> = Vec::new();

    for (i, row) in import_data.rows.iter().enumerate() {
        let row_num = i + 2;

        if import_data.data_type == "holdings" {
            let symbol = extract_str(row, "symbol").to_uppercase();
            let name = extract_str(row, "name").to_string();
            let market = {
                let m = extract_str(row, "market");
                if m.is_empty() { "US" } else { m }.to_string()
            };
            let shares: f64 = extract_str(row, "shares").parse().unwrap_or(0.0);
            let avg_cost: f64 = extract_str(row, "avg_cost").parse().unwrap_or(0.0);
            let currency = {
                let c = extract_str(row, "currency");
                if c.is_empty() {
                    if market == "CN" { "CNY" } else if market == "HK" { "HKD" } else { "USD" }
                } else {
                    c
                }
                .to_string()
            };

            if symbol.is_empty() {
                errors.push(ImportError {
                    row: row_num,
                    column: "symbol".to_string(),
                    message: format!("第{}行 symbol 为空", row_num),
                });
                skipped_count += 1;
                continue;
            }

            // Check for duplicate holdings in the same account
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM holdings WHERE symbol = ?1 AND account_id = ?2",
                    rusqlite::params![symbol, import_data.account_id],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);

            if exists {
                skipped_count += 1;
                continue;
            }

            let id = Uuid::new_v4().to_string();
            match conn.execute(
                "INSERT INTO holdings (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![id, import_data.account_id, symbol, name, market, shares, avg_cost, currency, now, now],
            ) {
                Ok(_) => imported_count += 1,
                Err(e) => {
                    errors.push(ImportError {
                        row: row_num,
                        column: String::new(),
                        message: e.to_string(),
                    });
                    skipped_count += 1;
                }
            }
        } else if import_data.data_type == "transactions" {
            let symbol = extract_str(row, "symbol").to_uppercase();
            let name = extract_str(row, "name").to_string();
            let market = {
                let m = extract_str(row, "market");
                if m.is_empty() { "US" } else { m }.to_string()
            };
            let tx_type = extract_str(row, "transaction_type").to_uppercase();
            let traded_at = extract_str(row, "traded_at").to_string();
            let shares: f64 = extract_str(row, "shares").parse().unwrap_or(0.0);
            let price: f64 = extract_str(row, "price").parse().unwrap_or(0.0);
            let commission: f64 = extract_str(row, "commission").parse().unwrap_or(0.0);
            let currency = {
                let c = extract_str(row, "currency");
                if c.is_empty() { "USD" } else { c }.to_string()
            };
            let notes: Option<String> = {
                let n = extract_str(row, "notes").to_string();
                if n.is_empty() { None } else { Some(n) }
            };
            let total_amount = shares * price;

            if symbol.is_empty() || traded_at.is_empty() {
                skipped_count += 1;
                continue;
            }

            let id = Uuid::new_v4().to_string();
            match conn.execute(
                "INSERT INTO transactions (id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                rusqlite::params![id, import_data.account_id, symbol, name, market, tx_type, shares, price, total_amount, commission, currency, traded_at, notes, now],
            ) {
                Ok(_) => imported_count += 1,
                Err(e) => {
                    errors.push(ImportError {
                        row: row_num,
                        column: String::new(),
                        message: e.to_string(),
                    });
                    skipped_count += 1;
                }
            }
        }
    }

    Ok(ImportResult {
        imported_count,
        skipped_count,
        errors,
    })
}
