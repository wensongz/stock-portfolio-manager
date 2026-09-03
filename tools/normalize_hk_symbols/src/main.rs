//! Standalone utility: normalize_hk_symbols
//!
//! 用途：将数据库所有表中港股（市场 = HK）的股票代码统一去掉前导零。
//!       例如 0998.HK → 998.HK，0941.HK → 941.HK。
//!
//! 涉及的表：
//!   - holdings                  (symbol, market)
//!   - transactions              (symbol, market)
//!   - daily_holding_snapshots   (symbol, market)
//!   - quarterly_holding_snapshots (symbol, market)
//!   - price_alerts              (symbol, market)
//!   - benchmark_daily_prices    (symbol，无 market 列，按 %.HK 匹配)
//!   - cached_quotes             (symbol 为主键，需要删除旧行，插入新行)
//!
//! 用法：
//!   cargo run -- <数据库路径> [--dry-run]
//!
//! 选项：
//!   --dry-run   仅打印将要执行的操作，不写入数据库。

use rusqlite::{params, Connection};

/// Strip leading zeros from an HK symbol, e.g. "0998.HK" → "998.HK".
/// Returns None if the symbol does not need normalization.
fn normalize_hk_symbol(symbol: &str) -> Option<String> {
    let suffix = ".HK";
    if !symbol.ends_with(suffix) {
        return None;
    }
    let code = &symbol[..symbol.len() - suffix.len()];
    // Remove leading zeros; keep at least one digit.
    let normalized_code = code.trim_start_matches('0').max("0");
    let normalized = format!("{}{}", normalized_code, suffix);
    if normalized == symbol {
        None // already normalized
    } else {
        Some(normalized)
    }
}

struct TableSpec {
    /// Table name
    table: &'static str,
    /// Column name that holds the symbol
    symbol_col: &'static str,
    /// Optional market column name; if Some, the WHERE clause also filters by market='HK'
    market_col: Option<&'static str>,
    /// Whether symbol is the PRIMARY KEY (requires DELETE+INSERT instead of UPDATE)
    symbol_is_pk: bool,
}

/// Row data for the `cached_quotes` table (needed for DELETE + re-INSERT on PRIMARY KEY rename).
struct CachedQuoteRow {
    name: String,
    market: String,
    current_price: f64,
    previous_close: f64,
    change: f64,
    change_percent: f64,
    high: f64,
    low: f64,
    volume: i64,
    updated_at: String,
    pe_ttm: Option<f64>,
    pb: Option<f64>,
    market_cap: Option<f64>,
    dividend_yield: Option<f64>,
    eps: Option<f64>,
    roe: Option<f64>,
    turnover_rate: Option<f64>,
}

const CACHED_QUOTE_METADATA_COLUMNS: &[&str] = &[
    "pe_ttm",
    "pb",
    "market_cap",
    "dividend_yield",
    "eps",
    "roe",
    "turnover_rate",
];

/// Refuse to mutate a pre-v3 database. This check runs before the table loop so
/// the utility cannot leave symbols normalized in only a subset of tables.
fn ensure_cached_quotes_metadata_schema(conn: &Connection) -> Result<(), String> {
    let table_exists: bool = conn
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'cached_quotes'
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !table_exists {
        return Ok(());
    }

    for column in CACHED_QUOTE_METADATA_COLUMNS {
        let column_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM pragma_table_info('cached_quotes') WHERE name = ?1
                 )",
                params![column],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !column_exists {
            return Err(format!(
                "cached_quotes 缺少字段 {column}；请先用当前版本应用打开数据库完成 v3 迁移"
            ));
        }
    }
    Ok(())
}

fn replace_cached_quote_symbol(
    conn: &Connection,
    old_symbol: &str,
    new_symbol: &str,
) -> rusqlite::Result<usize> {
    let transaction = conn.unchecked_transaction()?;
    let rows: Vec<CachedQuoteRow> = {
        let mut stmt = transaction.prepare(
            "SELECT name, market, current_price, previous_close,
                    change, change_percent, high, low, volume, updated_at,
                    pe_ttm, pb, market_cap, dividend_yield, eps, roe, turnover_rate
             FROM cached_quotes WHERE symbol = ?1",
        )?;
        let mapped_rows = stmt.query_map(params![old_symbol], |row| {
            Ok(CachedQuoteRow {
                name: row.get(0)?,
                market: row.get(1)?,
                current_price: row.get(2)?,
                previous_close: row.get(3)?,
                change: row.get(4)?,
                change_percent: row.get(5)?,
                high: row.get(6)?,
                low: row.get(7)?,
                volume: row.get(8)?,
                updated_at: row.get(9)?,
                pe_ttm: row.get(10)?,
                pb: row.get(11)?,
                market_cap: row.get(12)?,
                dividend_yield: row.get(13)?,
                eps: row.get(14)?,
                roe: row.get(15)?,
                turnover_rate: row.get(16)?,
            })
        })?;
        mapped_rows.collect::<rusqlite::Result<_>>()?
    };

    for row in &rows {
        transaction.execute(
            "INSERT OR REPLACE INTO cached_quotes
             (symbol, name, market, current_price, previous_close,
              change, change_percent, high, low, volume, updated_at,
              pe_ttm, pb, market_cap, dividend_yield, eps, roe, turnover_rate)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![
                new_symbol,
                row.name,
                row.market,
                row.current_price,
                row.previous_close,
                row.change,
                row.change_percent,
                row.high,
                row.low,
                row.volume,
                row.updated_at,
                row.pe_ttm,
                row.pb,
                row.market_cap,
                row.dividend_yield,
                row.eps,
                row.roe,
                row.turnover_rate,
            ],
        )?;
    }

    let deleted = transaction.execute(
        "DELETE FROM cached_quotes WHERE symbol = ?1",
        params![old_symbol],
    )?;
    transaction.commit()?;
    Ok(deleted)
}

const TABLES: &[TableSpec] = &[
    TableSpec {
        table: "holdings",
        symbol_col: "symbol",
        market_col: Some("market"),
        symbol_is_pk: false,
    },
    TableSpec {
        table: "transactions",
        symbol_col: "symbol",
        market_col: Some("market"),
        symbol_is_pk: false,
    },
    TableSpec {
        table: "daily_holding_snapshots",
        symbol_col: "symbol",
        market_col: Some("market"),
        symbol_is_pk: false,
    },
    TableSpec {
        table: "quarterly_holding_snapshots",
        symbol_col: "symbol",
        market_col: Some("market"),
        symbol_is_pk: false,
    },
    TableSpec {
        table: "price_alerts",
        symbol_col: "symbol",
        market_col: Some("market"),
        symbol_is_pk: false,
    },
    TableSpec {
        table: "benchmark_daily_prices",
        symbol_col: "symbol",
        market_col: None, // no market column; match by %.HK suffix
        symbol_is_pk: false,
    },
    TableSpec {
        table: "cached_quotes",
        symbol_col: "symbol",
        market_col: Some("market"),
        symbol_is_pk: true, // PRIMARY KEY — must DELETE + re-INSERT
    },
];

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut db_path: Option<String> = None;
    let mut dry_run = false;
    let mut extra_paths = false;

    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            path => {
                if db_path.is_some() {
                    extra_paths = true;
                }
                db_path = Some(path.to_string());
            }
        }
    }

    if extra_paths {
        eprintln!("错误：提供了多个数据库路径，请只指定一个。");
        std::process::exit(1);
    }

    let db_path = db_path.unwrap_or_else(|| {
        eprintln!("用法: normalize_hk_symbols <数据库路径> [--dry-run]");
        eprintln!();
        eprintln!("  --dry-run   仅预览将要执行的操作，不写入数据库");
        eprintln!();
        eprintln!("示例:");
        eprintln!("  cargo run -- ~/Library/Application\\ Support/com.stock-portfolio-manager.app/portfolio.db");
        eprintln!("  cargo run -- ~/portfolio.db --dry-run");
        std::process::exit(1);
    });

    if dry_run {
        println!("=== DRY-RUN 模式（不写入数据库）===\n");
    }

    let conn = Connection::open(&db_path).unwrap_or_else(|e| {
        eprintln!("无法打开数据库 {}: {}", db_path, e);
        std::process::exit(1);
    });
    if let Err(error) = ensure_cached_quotes_metadata_schema(&conn) {
        eprintln!("数据库版本不兼容：{}", error);
        std::process::exit(1);
    }

    let mut total_updated = 0u64;
    let mut total_skipped = 0u64;

    for spec in TABLES {
        // Check whether the table exists (some databases may be older versions).
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![spec.table],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if !exists {
            println!("[跳过] 表 {} 不存在，忽略。", spec.table);
            continue;
        }

        // Build query to find HK symbols with leading zeros.
        let where_clause = match spec.market_col {
            Some(mc) => format!(
                "{mc} = 'HK' AND {sc} LIKE '0%.HK'",
                mc = mc,
                sc = spec.symbol_col
            ),
            None => format!("{sc} LIKE '0%.HK'", sc = spec.symbol_col),
        };

        let query = format!(
            "SELECT DISTINCT {sc} FROM {t} WHERE {w}",
            sc = spec.symbol_col,
            t = spec.table,
            w = where_clause
        );

        let affected_symbols: Vec<String> = {
            let mut stmt = conn.prepare(&query).unwrap_or_else(|e| {
                eprintln!("准备查询失败 [{}]: {}", spec.table, e);
                std::process::exit(1);
            });
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap_or_else(|e| {
                    eprintln!("查询失败 [{}]: {}", spec.table, e);
                    std::process::exit(1);
                })
                .filter_map(|r| r.ok())
                .collect()
        };

        if affected_symbols.is_empty() {
            println!("[{}] 无需处理（没有带前导零的港股代码）", spec.table);
            total_skipped += 1;
            continue;
        }

        for old_sym in &affected_symbols {
            let new_sym = match normalize_hk_symbol(old_sym) {
                Some(s) => s,
                None => {
                    println!("[{}] {} 已规范化，跳过", spec.table, old_sym);
                    total_skipped += 1;
                    continue;
                }
            };

            if dry_run {
                if spec.symbol_is_pk {
                    println!(
                        "[预览] 表 {}: {} → {} （主键：将删除旧行并插入新行）",
                        spec.table, old_sym, new_sym
                    );
                } else {
                    println!("[预览] 表 {}: {} → {}", spec.table, old_sym, new_sym);
                }
                total_updated += 1;
                continue;
            }

            if spec.symbol_is_pk {
                // For PRIMARY KEY columns we cannot UPDATE directly.
                // Strategy: copy the row(s) with the new symbol (INSERT OR REPLACE),
                // then delete the old row(s).
                match replace_cached_quote_symbol(&conn, old_sym, &new_sym) {
                    Ok(_) => println!(
                        "[更新] 表 {}: {} → {} （主键替换）",
                        spec.table, old_sym, new_sym
                    ),
                    Err(error) => {
                        eprintln!(
                            "主键替换失败 [{}] {} → {}: {}",
                            spec.table, old_sym, new_sym, error
                        );
                        total_skipped += 1;
                        continue;
                    }
                }
            } else {
                let update_sql = format!(
                    "UPDATE {t} SET {sc} = ?1 WHERE {sc} = ?2{market_filter}",
                    t = spec.table,
                    sc = spec.symbol_col,
                    market_filter = match spec.market_col {
                        Some(mc) => format!(" AND {} = 'HK'", mc),
                        None => String::new(),
                    }
                );

                let rows_changed = conn
                    .execute(&update_sql, params![new_sym, old_sym])
                    .unwrap_or_else(|e| {
                        eprintln!("更新失败 [{}] {} → {}: {}", spec.table, old_sym, new_sym, e);
                        0
                    });

                println!(
                    "[更新] 表 {}: {} → {} （影响 {} 行）",
                    spec.table, old_sym, new_sym, rows_changed
                );
            }

            total_updated += 1;
        }
    }

    println!();
    println!("=== 汇总 ===");
    if dry_run {
        println!("将更新（预览）: {}", total_updated);
        println!("无需处理:       {}", total_skipped);
        println!();
        println!("以上为预览结果。去掉 --dry-run 参数后再次运行即可写入数据库。");
    } else {
        println!("已更新: {}", total_updated);
        println!("跳过:   {}", total_skipped);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_cached_quotes_metadata_schema, normalize_hk_symbol, replace_cached_quote_symbol,
    };
    use rusqlite::{params, Connection};

    type OptionalMetadata = (
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
    );

    #[test]
    fn test_normalize_hk_symbol() {
        assert_eq!(normalize_hk_symbol("0998.HK"), Some("998.HK".to_string()));
        assert_eq!(normalize_hk_symbol("00941.HK"), Some("941.HK".to_string()));
        assert_eq!(normalize_hk_symbol("0700.HK"), Some("700.HK".to_string()));
        assert_eq!(normalize_hk_symbol("00001.HK"), Some("1.HK".to_string()));
        // Already normalized — should return None
        assert_eq!(normalize_hk_symbol("998.HK"), None);
        assert_eq!(normalize_hk_symbol("941.HK"), None);
        // Non-HK symbols — should return None
        assert_eq!(normalize_hk_symbol("SH600036"), None);
        assert_eq!(normalize_hk_symbol("AAPL"), None);
    }

    #[test]
    fn replacing_cached_quote_symbol_preserves_optional_metadata() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE cached_quotes (
                   symbol TEXT PRIMARY KEY NOT NULL,
                   name TEXT NOT NULL,
                   market TEXT NOT NULL,
                   current_price REAL NOT NULL,
                   previous_close REAL NOT NULL,
                   change REAL NOT NULL,
                   change_percent REAL NOT NULL,
                   high REAL NOT NULL,
                   low REAL NOT NULL,
                   volume INTEGER NOT NULL,
                   updated_at TEXT NOT NULL,
                   pe_ttm REAL,
                   pb REAL,
                   market_cap REAL,
                   dividend_yield REAL,
                   eps REAL,
                   roe REAL,
                   turnover_rate REAL
                 );
                 INSERT INTO cached_quotes VALUES
                   ('00700.HK', '腾讯控股', 'HK', 620, 615, 5, 0.81, 623, 610,
                    998877, '2026-09-03T00:00:00Z', 22.5, 4.2, 5800000000000,
                    0.7, 28.1, 19.5, 0.61);",
            )
            .unwrap();

        replace_cached_quote_symbol(&connection, "00700.HK", "700.HK").unwrap();

        let metadata: OptionalMetadata = connection
            .query_row(
                "SELECT pe_ttm, pb, market_cap, dividend_yield, eps, roe, turnover_rate
                 FROM cached_quotes WHERE symbol = ?1",
                params!["700.HK"],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            metadata,
            (
                Some(22.5),
                Some(4.2),
                Some(5_800_000_000_000.0),
                Some(0.7),
                Some(28.1),
                Some(19.5),
                Some(0.61),
            )
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM cached_quotes WHERE symbol = '00700.HK'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn metadata_schema_preflight_rejects_v2_without_writing() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE cached_quotes (
                   symbol TEXT PRIMARY KEY NOT NULL,
                   name TEXT NOT NULL,
                   market TEXT NOT NULL,
                   current_price REAL NOT NULL,
                   previous_close REAL NOT NULL,
                   change REAL NOT NULL,
                   change_percent REAL NOT NULL,
                   high REAL NOT NULL,
                   low REAL NOT NULL,
                   volume INTEGER NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                 INSERT INTO cached_quotes VALUES
                   ('00700.HK', '腾讯控股', 'HK', 620, 615, 5, 0.81, 623, 610,
                    998877, '2026-09-03T00:00:00Z');",
            )
            .unwrap();

        let error = ensure_cached_quotes_metadata_schema(&connection).unwrap_err();
        assert!(error.contains("v3"));
        assert_eq!(
            connection
                .query_row("SELECT symbol FROM cached_quotes", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            "00700.HK"
        );
    }
}
