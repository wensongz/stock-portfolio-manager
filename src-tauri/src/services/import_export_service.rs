use crate::db::Database;
use crate::models::import_export::{
    ExportFilters, ImportError, ImportPreview, ImportResult, ImportSkipped,
};
use crate::services::portfolio_mutation::{
    create_holding_in, create_transaction_in, CreateHoldingInput, CreateTransactionInput,
};
use csv::WriterBuilder;
use std::collections::{HashMap, HashSet};

// ─────────────────────────────────────────────────────────────────────────────
// Export
// ─────────────────────────────────────────────────────────────────────────────

struct HoldingExportRow {
    account_name: Option<String>,
    symbol: String,
    name: String,
    market: String,
    category_name: Option<String>,
    shares: f64,
    avg_cost: f64,
    currency: String,
}

impl HoldingExportRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            account_name: row.get(0)?,
            symbol: row.get(1)?,
            name: row.get(2)?,
            market: row.get(3)?,
            category_name: row.get(4)?,
            shares: row.get(5)?,
            avg_cost: row.get(6)?,
            currency: row.get(7)?,
        })
    }

    fn into_record(self) -> [String; 8] {
        [
            self.account_name.unwrap_or_default(),
            self.symbol,
            self.name,
            self.market,
            self.category_name.unwrap_or_default(),
            self.shares.to_string(),
            self.avg_cost.to_string(),
            self.currency,
        ]
    }
}

struct TransactionExportRow {
    traded_at: String,
    account_name: Option<String>,
    symbol: String,
    name: String,
    market: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
    commission: f64,
    currency: String,
    notes: Option<String>,
}

impl TransactionExportRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            traded_at: row.get(0)?,
            account_name: row.get(1)?,
            symbol: row.get(2)?,
            name: row.get(3)?,
            market: row.get(4)?,
            transaction_type: row.get(5)?,
            shares: row.get(6)?,
            price: row.get(7)?,
            total_amount: row.get(8)?,
            commission: row.get(9)?,
            currency: row.get(10)?,
            notes: row.get(11)?,
        })
    }

    fn into_record(self) -> [String; 12] {
        [
            self.traded_at,
            self.account_name.unwrap_or_default(),
            self.symbol,
            self.name,
            self.market,
            self.transaction_type,
            self.shares.to_string(),
            self.price.to_string(),
            self.total_amount.to_string(),
            self.commission.to_string(),
            self.currency,
            self.notes.unwrap_or_default(),
        ]
    }
}

/// Export holdings to CSV and return the CSV string content.
pub fn export_holdings_csv(db: &Database, filters: &ExportFilters) -> Result<String, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

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
    wtr.write_record([
        "账户名",
        "股票代码",
        "股票名称",
        "市场",
        "类别",
        "持仓数量",
        "均价",
        "币种",
    ])
    .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter()),
            HoldingExportRow::from_row,
        )
        .map_err(|e| e.to_string())?;

    for row in rows {
        wtr.write_record(row.map_err(|e| e.to_string())?.into_record())
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
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

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
    wtr.write_record([
        "交易日期",
        "账户名",
        "股票代码",
        "股票名称",
        "市场",
        "买卖方向",
        "数量",
        "价格",
        "金额",
        "手续费",
        "币种",
        "备注",
    ])
    .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter()),
            TransactionExportRow::from_row,
        )
        .map_err(|e| e.to_string())?;

    for row in rows {
        wtr.write_record(row.map_err(|e| e.to_string())?.into_record())
            .map_err(|e| e.to_string())?;
    }

    let data = wtr.into_inner().map_err(|e| e.to_string())?;
    String::from_utf8(data).map_err(|e| e.to_string())
}

/// Generate holdings import template CSV.
pub fn get_holdings_template() -> String {
    let mut wtr = WriterBuilder::new().from_writer(vec![]);
    let _ = wtr.write_record(["symbol", "name", "market", "shares", "avg_cost", "currency"]);
    let _ = wtr.write_record(["AAPL", "苹果", "US", "100", "150.00", "USD"]);
    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

/// Generate transactions import template CSV.
pub fn get_transactions_template() -> String {
    let mut wtr = WriterBuilder::new().from_writer(vec![]);
    let _ = wtr.write_record([
        "traded_at",
        "symbol",
        "name",
        "market",
        "transaction_type",
        "shares",
        "price",
        "total_amount",
        "commission",
        "currency",
        "notes",
    ]);
    let _ = wtr.write_record([
        "2024-01-15",
        "AAPL",
        "苹果",
        "US",
        "BUY",
        "100",
        "150.00",
        "",
        "0",
        "USD",
        "",
    ]);
    let _ = wtr.write_record([
        "2024-03-01",
        "AAPL",
        "苹果",
        "US",
        "PAY",
        "0",
        "0",
        "50.00",
        "0",
        "USD",
        "分红派息",
    ]);
    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// Import / Parse
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ParsedImportRow {
    row_number: usize,
    data: serde_json::Value,
}

#[derive(Debug)]
struct ParsedImport {
    preview: ImportPreview,
    valid_rows: Vec<ParsedImportRow>,
}

fn parse_import_rows(content: &str, data_type: &str) -> Result<ParsedImport, String> {
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

    let required_fields: &[&str] = match data_type {
        "holdings" => &required_holdings,
        "transactions" => &required_transactions,
        _ => return Err(format!("不支持的导入数据类型: {data_type}")),
    };

    let column_mapping: HashMap<String, String> =
        headers.iter().map(|h| (h.clone(), h.clone())).collect();

    let mut preview_data: Vec<serde_json::Value> = Vec::new();
    let mut valid_import_rows: Vec<ParsedImportRow> = Vec::new();
    let mut error_rows: Vec<ImportError> = Vec::new();
    let mut invalid_row_numbers = HashSet::new();
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
                invalid_row_numbers.insert(row_num);
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
                invalid_row_numbers.insert(row_num);
                has_error = true;
            }
        }

        if !has_error {
            let row = serde_json::Value::Object(row_map);
            if preview_data.len() < 20 {
                preview_data.push(row.clone());
            }
            valid_import_rows.push(ParsedImportRow {
                row_number: row_num,
                data: row,
            });
        }
    }

    let valid_rows = total_rows.saturating_sub(invalid_row_numbers.len());

    Ok(ParsedImport {
        preview: ImportPreview {
            total_rows,
            valid_rows,
            error_rows,
            preview_data,
            column_mapping,
        },
        valid_rows: valid_import_rows,
    })
}

/// Parse CSV content and return an ImportPreview (validate but don't write).
pub fn parse_import_csv(content: &str, data_type: &str) -> Result<ImportPreview, String> {
    Ok(parse_import_rows(content, data_type)?.preview)
}

/// Extract a string field value from a JSON row.
fn extract_str<'a>(row: &'a serde_json::Value, key: &str) -> &'a str {
    row.get(key).and_then(|v| v.as_str()).unwrap_or("").trim()
}

fn parse_number(row: &serde_json::Value, key: &str, row_number: usize) -> Result<f64, ImportError> {
    let raw = extract_str(row, key);
    raw.parse::<f64>().map_err(|_| ImportError {
        row: row_number,
        column: key.to_string(),
        message: format!("第{row_number}行 {key} 必须是有效数字"),
    })
}

fn parse_optional_number(
    row: &serde_json::Value,
    key: &str,
    row_number: usize,
) -> Result<f64, ImportError> {
    let raw = extract_str(row, key);
    if raw.is_empty() {
        Ok(0.0)
    } else {
        parse_number(row, key, row_number)
    }
}

/// Reparse and write every valid CSV row to the database.
pub fn confirm_import(
    db: &Database,
    content: &str,
    data_type: &str,
    account_id: &str,
) -> Result<ImportResult, String> {
    let parsed = parse_import_rows(content, data_type)?;
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut result = ImportResult {
        imported_count: 0,
        skipped_count: 0,
        skipped_rows: Vec::new(),
        errors: Vec::new(),
    };

    for parsed_row in &parsed.valid_rows {
        let row = &parsed_row.data;
        let row_number = parsed_row.row_number;
        let symbol = extract_str(row, "symbol").to_uppercase();

        let mutation_result = if data_type == "holdings" {
            let market = match extract_str(row, "market") {
                "" => "US".to_string(),
                value => value.to_string(),
            };
            let currency = match extract_str(row, "currency") {
                "" if market == "CN" => "CNY".to_string(),
                "" if market == "HK" => "HKD".to_string(),
                "" => "USD".to_string(),
                value => value.to_string(),
            };
            let values = parse_number(row, "shares", row_number).and_then(|shares| {
                parse_number(row, "avg_cost", row_number).map(|avg_cost| (shares, avg_cost))
            });
            match values {
                Ok((shares, avg_cost)) => {
                    let input = CreateHoldingInput {
                        account_id: account_id.to_string(),
                        symbol: symbol.clone(),
                        name: extract_str(row, "name").to_string(),
                        market,
                        category_id: None,
                        shares,
                        avg_cost,
                        currency,
                    };
                    let mut savepoint = conn.savepoint().map_err(|error| error.to_string())?;
                    match create_holding_in(&savepoint, &input) {
                        Ok(_) => savepoint
                            .commit()
                            .map(|_| ())
                            .map_err(|error| (String::new(), error.to_string())),
                        Err(error) => {
                            savepoint.rollback().map_err(|rollback_error| {
                                format!("导入回滚失败: {rollback_error}")
                            })?;
                            Err((String::new(), error))
                        }
                    }
                }
                Err(error) => Err((error.column, error.message)),
            }
        } else {
            let market = match extract_str(row, "market") {
                "" => "US".to_string(),
                value => value.to_string(),
            };
            let currency = match extract_str(row, "currency") {
                "" if market == "CN" => "CNY".to_string(),
                "" if market == "HK" => "HKD".to_string(),
                "" => "USD".to_string(),
                value => value.to_string(),
            };
            let values = parse_number(row, "shares", row_number).and_then(|shares| {
                parse_number(row, "price", row_number).and_then(|price| {
                    parse_optional_number(row, "commission", row_number).and_then(|commission| {
                        parse_optional_number(row, "total_amount", row_number)
                            .map(|amount| (shares, price, commission, amount))
                    })
                })
            });
            match values {
                Ok((shares, price, commission, amount)) => {
                    let input = CreateTransactionInput {
                        account_id: account_id.to_string(),
                        symbol: symbol.clone(),
                        name: extract_str(row, "name").to_string(),
                        market,
                        transaction_type: extract_str(row, "transaction_type").to_uppercase(),
                        shares,
                        price,
                        total_amount: if amount.abs() > 0.0 {
                            amount
                        } else {
                            shares * price
                        },
                        commission,
                        currency,
                        traded_at: extract_str(row, "traded_at").to_string(),
                        notes: match extract_str(row, "notes") {
                            "" => None,
                            notes => Some(notes.to_string()),
                        },
                    };
                    let mut savepoint = conn.savepoint().map_err(|error| error.to_string())?;
                    match create_transaction_in(&savepoint, &input) {
                        Ok(_) => savepoint
                            .commit()
                            .map(|_| ())
                            .map_err(|error| (String::new(), error.to_string())),
                        Err(error) => {
                            savepoint.rollback().map_err(|rollback_error| {
                                format!("导入回滚失败: {rollback_error}")
                            })?;
                            Err((String::new(), error))
                        }
                    }
                }
                Err(error) => Err((error.column, error.message)),
            }
        };

        match mutation_result {
            Ok(()) => result.imported_count += 1,
            Err((column, message)) => {
                result.skipped_count += 1;
                result.errors.push(ImportError {
                    row: row_number,
                    column,
                    message: message.clone(),
                });
                result.skipped_rows.push(ImportSkipped {
                    row: row_number,
                    symbol: if symbol.is_empty() {
                        "(空)".to_string()
                    } else {
                        symbol
                    },
                    reason: message,
                });
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_with_account() -> Database {
        let db = Database::new(":memory:").expect("in-memory database");
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES ('account-1', 'Import', 'US', '2026-08-31', '2026-08-31')",
                [],
            )
            .unwrap();
        }
        db
    }

    fn holdings_csv(row_count: usize) -> String {
        let mut csv = String::from("symbol,name,market,shares,avg_cost,currency\n");
        for index in 1..=row_count {
            csv.push_str(&format!("TEST{index:02},Test {index},US,{index},10,USD\n"));
        }
        csv
    }

    fn no_filters() -> ExportFilters {
        ExportFilters {
            market: None,
            account_id: None,
            category_id: None,
        }
    }

    #[test]
    fn holdings_export_rejects_invalid_numeric_database_values() {
        let db = database_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, shares, avg_cost, currency,
                  created_at, updated_at)
                 VALUES ('broken-holding', 'account-1', 'BROKEN', 'Broken', 'US',
                         'not-a-number', 10, 'USD', '2026-09-03', '2026-09-03')",
                [],
            )
            .unwrap();
        }

        let error = export_holdings_csv(&db, &no_filters()).unwrap_err();
        assert!(error.contains("Invalid column type"), "{error}");
    }

    #[test]
    fn transactions_export_rejects_invalid_numeric_database_values() {
        let db = database_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO transactions
                 (id, holding_id, account_id, symbol, name, market, transaction_type,
                  shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES ('broken-transaction', NULL, 'account-1', 'BROKEN', 'Broken', 'US',
                         'BUY', 1, 'not-a-number', 10, 0, 'USD', '2026-09-03', NULL,
                         '2026-09-03')",
                [],
            )
            .unwrap();
        }

        let error = export_transactions_csv(&db, "", "", &no_filters()).unwrap_err();
        assert!(error.contains("Invalid column type"), "{error}");
    }

    #[test]
    fn nullable_export_fields_remain_blank_cells() {
        let db = database_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('holding-1', 'account-1', 'AAPL', 'Apple', 'US', NULL, 2, 10,
                         'USD', '2026-09-03', '2026-09-03');
                 INSERT INTO transactions
                 (id, holding_id, account_id, symbol, name, market, transaction_type,
                  shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES ('transaction-1', 'holding-1', 'account-1', 'AAPL', 'Apple', 'US',
                         'BUY', 2, 10, 20, 0, 'USD', '2026-09-03', NULL, '2026-09-03');",
            )
            .unwrap();
        }

        let holdings = export_holdings_csv(&db, &no_filters()).unwrap();
        let holding_row = csv::Reader::from_reader(holdings.as_bytes())
            .records()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(&holding_row[4], "");

        let transactions = export_transactions_csv(&db, "", "", &no_filters()).unwrap();
        let transaction_row = csv::Reader::from_reader(transactions.as_bytes())
            .records()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(&transaction_row[11], "");
    }

    #[test]
    fn confirmation_imports_all_valid_rows_beyond_preview_limit() {
        let db = database_with_account();
        let csv = holdings_csv(25);
        let parsed = parse_import_rows(&csv, "holdings").unwrap();
        assert_eq!(parsed.preview.total_rows, 25);
        assert_eq!(parsed.preview.valid_rows, 25);
        assert_eq!(parsed.preview.preview_data.len(), 20);
        assert_eq!(parsed.valid_rows.len(), 25);

        let result = confirm_import(&db, &csv, "holdings", "account-1").unwrap();

        assert_eq!(result.imported_count, 25);
        let holding_count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM holdings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(holding_count, 25);
    }

    #[test]
    fn valid_row_count_counts_invalid_rows_once_when_multiple_fields_are_missing() {
        let csv = concat!(
            "symbol,name,market,shares,avg_cost,currency\n",
            "AAPL,Apple,US,10,100,USD\n",
            ",Broken,US,,100,USD\n",
        );

        let preview = parse_import_csv(csv, "holdings").unwrap();

        assert_eq!(preview.total_rows, 2);
        assert_eq!(preview.valid_rows, 1);
        assert_eq!(preview.error_rows.len(), 2);
        assert!(preview.error_rows.iter().all(|error| error.row == 3));
    }

    #[test]
    fn parser_rejects_unsupported_data_types() {
        let error = parse_import_csv(
            "traded_at,symbol,transaction_type,shares,price\n2026-01-01,AAPL,BUY,1,10\n",
            "unknown",
        )
        .unwrap_err();

        assert!(error.contains("不支持的导入数据类型"));
    }

    #[test]
    fn imported_holding_creates_open_baseline_transaction() {
        let db = database_with_account();
        let csv = holdings_csv(1);

        let result = confirm_import(&db, &csv, "holdings", "account-1").unwrap();

        assert_eq!(result.imported_count, 1);
        let transaction_type: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT transaction_type FROM transactions WHERE symbol = 'TEST01'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(transaction_type, "OPEN");
    }

    #[test]
    fn imported_buy_applies_the_same_cash_impact_as_a_normal_transaction() {
        let db = database_with_account();
        let csv = concat!(
            "traded_at,symbol,name,market,transaction_type,shares,price,total_amount,commission,currency,notes\n",
            "2026-08-31,AAPL,Apple,US,BUY,2,50,100,3,USD,\n",
        );

        let result = confirm_import(&db, csv, "transactions", "account-1").unwrap();

        assert_eq!(result.imported_count, 1);
        let cash: f64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT shares FROM holdings WHERE symbol = '$CASH-USD'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        assert_eq!(cash, -103.0);
    }

    #[test]
    fn imported_sell_uses_commission_adjusted_net_proceeds_for_cost_basis() {
        let db = database_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
                 VALUES ('holding-cn', 'account-1', 'sh600000', '浦发银行', 'CN', 10, 100, 'CNY', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
        }
        let csv = concat!(
            "traded_at,symbol,name,market,transaction_type,shares,price,total_amount,commission,currency,notes\n",
            "2026-08-31,sh600000,浦发银行,CN,SELL,2,120,240,5,CNY,\n",
        );

        let result = confirm_import(&db, csv, "transactions", "account-1").unwrap();

        assert_eq!(result.imported_count, 1);
        let avg_cost: f64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT avg_cost FROM holdings WHERE id = 'holding-cn'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((avg_cost - 95.625).abs() < 1e-12, "got {avg_cost}");
    }

    #[test]
    fn failed_import_row_rolls_back_holding_changes() {
        let db = database_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_transaction_insert
                 BEFORE INSERT ON transactions
                 WHEN NEW.symbol = 'FAIL'
                 BEGIN
                   SELECT RAISE(ABORT, 'forced transaction failure');
                 END;",
            )
            .unwrap();
        }
        let csv = concat!(
            "traded_at,symbol,name,market,transaction_type,shares,price,total_amount,commission,currency,notes\n",
            "2026-08-31,FAIL,Failure,US,BUY,2,50,100,0,USD,\n",
        );

        let result = confirm_import(&db, csv, "transactions", "account-1").unwrap();

        assert_eq!(result.imported_count, 0);
        assert_eq!(result.skipped_count, 1);
        let holding_count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM holdings WHERE symbol = 'FAIL'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(holding_count, 0);
    }

    #[test]
    fn invalid_numeric_holding_values_are_skipped_instead_of_becoming_zero() {
        let db = database_with_account();
        let csv = concat!(
            "symbol,name,market,shares,avg_cost,currency\n",
            "AAPL,Apple,US,not-a-number,100,USD\n",
        );

        let result = confirm_import(&db, csv, "holdings", "account-1").unwrap();

        assert_eq!(result.imported_count, 0);
        assert_eq!(result.skipped_count, 1);
        assert_eq!(result.errors[0].column, "shares");
    }
}
