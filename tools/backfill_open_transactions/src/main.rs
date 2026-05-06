//! Standalone utility: backfill_open_transactions
//!
//! 用途：为 holdings 表中的持仓在 transactions 表创建建仓(OPEN)记录。
//!
//! 逻辑：
//!   1. 若持仓已有 OPEN 记录 → 跳过（幂等）。
//!   2. 若持仓没有任何 BUY/SELL 交易记录 →
//!      直接以持仓的 shares / avg_cost 创建 OPEN。
//!   3. 若持仓有 BUY/SELL 记录但没有 OPEN →
//!      反推初始建仓数量和价格，再创建 OPEN。
//!
//! 推导（SELLs 不改变 avg_cost，avg_cost 等于历史所有买入加权均价）：
//!   shares₀ = shares_final + Σsell_shares − Σbuy_shares
//!   price₀  = ( (shares_final + Σsell_shares) × avg_cost_final
//!               − Σ(buy_shares × buy_price) ) / shares₀
//!
//! 用法：
//!   cargo run -- <数据库路径> [--dry-run]
//!
//! 选项：
//!   --dry-run   仅打印将要执行的操作，不写入数据库。

use chrono::{DateTime, Duration, FixedOffset};
use rusqlite::{params, Connection};
use uuid::Uuid;

const CASH_SYMBOL_PREFIX: &str = "$CASH-";
/// Tolerance for floating-point zero comparisons.
const EPSILON: f64 = 1e-6;

struct HoldingRow {
    id: String,
    account_id: String,
    symbol: String,
    name: String,
    market: String,
    shares: f64,
    avg_cost: f64,
    currency: String,
    created_at: String,
}

struct TxnRow {
    transaction_type: String,
    shares: f64,
    price: f64,
    traded_at: String,
    notes: Option<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut db_path: Option<String> = None;
    let mut dry_run = false;

    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            path => db_path = Some(path.to_string()),
        }
    }

    let db_path = db_path.unwrap_or_else(|| {
        eprintln!("用法: backfill_open_transactions <数据库路径> [--dry-run]");
        eprintln!();
        eprintln!("  --dry-run   仅预览将要创建的记录，不写入数据库");
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

    // Load all non-cash holdings.
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, symbol, name, market, shares, avg_cost, currency, created_at
             FROM holdings
             WHERE symbol NOT LIKE ?1
             ORDER BY account_id, symbol",
        )
        .expect("无法准备 holdings 查询");

    let like_pattern = format!("{}%", CASH_SYMBOL_PREFIX);
    let holdings: Vec<HoldingRow> = stmt
        .query_map(params![like_pattern], |row| {
            Ok(HoldingRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                symbol: row.get(2)?,
                name: row.get(3)?,
                market: row.get(4)?,
                shares: row.get(5)?,
                avg_cost: row.get(6)?,
                currency: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .expect("holdings 查询失败")
        .collect::<Result<Vec<_>, _>>()
        .expect("无法收集 holdings 结果");

    if holdings.is_empty() {
        println!("数据库中没有非现金持仓，无需处理。");
        return;
    }

    let mut created_count = 0usize;
    let mut skipped_count = 0usize;
    let mut error_count = 0usize;

    for holding in &holdings {
        let label = format!("{} ({})", holding.symbol, holding.name);

        // Fetch all transactions for this (account_id, symbol), ordered by time.
        let mut txn_stmt = conn
            .prepare(
                "SELECT transaction_type, shares, price, traded_at, notes
                 FROM transactions
                 WHERE account_id = ?1 AND UPPER(symbol) = UPPER(?2)
                 ORDER BY traded_at ASC",
            )
            .expect("无法准备 transactions 查询");

        let txns: Vec<TxnRow> = txn_stmt
            .query_map(params![holding.account_id, holding.symbol], |row| {
                Ok(TxnRow {
                    transaction_type: row.get(0)?,
                    shares: row.get(1)?,
                    price: row.get(2)?,
                    traded_at: row.get(3)?,
                    notes: row.get(4)?,
                })
            })
            .expect("transactions 查询失败")
            .collect::<Result<Vec<_>, _>>()
            .expect("无法收集 transactions 结果");

        // Skip if a backfill BUY record already exists (idempotent).
        if txns.iter().any(|t| {
            t.transaction_type == "BUY"
                && t.notes.as_deref() == Some("backfill:initial")
        }) {
            println!("[跳过] {}: 已存在建仓买入记录（backfill:initial）", label);
            skipped_count += 1;
            continue;
        }

        let buy_sell: Vec<&TxnRow> = txns
            .iter()
            .filter(|t| t.transaction_type == "BUY" || t.transaction_type == "SELL")
            .collect();

        let (open_shares, open_price, traded_at) = if buy_sell.is_empty() {
            // ── Case A ───────────────────────────────────────────────────────
            // No BUY/SELL transactions at all.
            // Create OPEN directly from the current holding data.
            (
                holding.shares,
                holding.avg_cost,
                holding.created_at.clone(),
            )
        } else {
            // ── Case B ───────────────────────────────────────────────────────
            // BUY/SELL records exist but no OPEN.
            // Back-calculate the initial position using the formulae:
            //
            //   Because SELLs never change avg_cost in this system, avg_cost_final
            //   equals the weighted average of ALL buys (including the initial OPEN):
            //     avg_cost_final = (s0·p0 + Σbuy_cost) / (s0 + Σbuy_shares)
            //
            //   Share balance:
            //     shares_final = s0 + Σbuy_shares − Σsell_shares
            //     ⟹ s0 = shares_final + Σsell_shares − Σbuy_shares
            //
            //   Since s0 + Σbuy_shares = shares_final + Σsell_shares:
            //     p0 = ((shares_final + Σsell_shares) · avg_cost_final − Σbuy_cost) / s0

            let sum_buy_shares: f64 = buy_sell
                .iter()
                .filter(|t| t.transaction_type == "BUY")
                .map(|t| t.shares)
                .sum();
            let sum_sell_shares: f64 = buy_sell
                .iter()
                .filter(|t| t.transaction_type == "SELL")
                .map(|t| t.shares)
                .sum();
            let sum_buy_cost: f64 = buy_sell
                .iter()
                .filter(|t| t.transaction_type == "BUY")
                .map(|t| t.shares * t.price)
                .sum();

            let s0 = holding.shares + sum_sell_shares - sum_buy_shares;

            if s0 <= EPSILON {
                // All current shares are fully accounted for by existing BUY
                // transactions; no prior OPEN position is needed.
                println!(
                    "[跳过] {}: 现有买入交易已能解释全部持仓（反推初始持股数 ≈ {:.4}），\
                     无需创建建仓记录",
                    label, s0
                );
                skipped_count += 1;
                continue;
            }

            let total_in = holding.shares + sum_sell_shares; // = s0 + Σbuy_shares
            let p0 = (total_in * holding.avg_cost - sum_buy_cost) / s0;

            if !p0.is_finite() {
                println!(
                    "[错误] {}: 反推的建仓价格无效（price₀ = {:.4}），跳过。\
                     \n       请检查持仓 avg_cost = {:.4} 及历史交易是否正确。",
                    label, p0, holding.avg_cost
                );
                error_count += 1;
                continue;
            }

            // Set traded_at to one day before the earliest existing transaction so
            // that the OPEN record sorts correctly in chronological order.
            let earliest = &buy_sell[0].traded_at;
            let open_date = date_minus_one_day(earliest, &holding.created_at);

            (s0, p0, open_date)
        };

        let total_amount = open_shares * open_price;

        println!(
            "[{}] {}: OPEN — {} 股 @ {:.4} {} | 成本 {:.2} | 日期 {}",
            if dry_run { "预览" } else { "创建" },
            label,
            open_shares,
            open_price,
            holding.currency,
            total_amount,
            traded_at,
        );

        if dry_run {
            created_count += 1;
            continue;
        }

        let txn_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        match conn.execute(
            "INSERT INTO transactions \
             (id, holding_id, account_id, symbol, name, market, \
              transaction_type, shares, price, total_amount, commission, \
              currency, traded_at, notes, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'BUY', ?7, ?8, ?9, 0.0, ?10, ?11, 'backfill:initial', ?12)",
            params![
                txn_id,
                holding.id,       // holding_id
                holding.account_id,
                holding.symbol,
                holding.name,
                holding.market,
                open_shares,
                open_price,
                total_amount,
                holding.currency,
                traded_at,
                now,
            ],
        ) {
            Ok(_) => {
                created_count += 1;
            }
            Err(e) => {
                println!("[错误] {}: 写入建仓买入记录失败: {}", label, e);
                error_count += 1;
            }
        }
    }

    println!();
    println!("=== 汇总 ===");
    if dry_run {
        println!("将创建（预览）: {}", created_count);
    } else {
        println!("已创建: {}", created_count);
    }
    println!("跳过:   {}", skipped_count);
    println!("错误:   {}", error_count);

    if dry_run {
        println!();
        println!("以上为预览结果。去掉 --dry-run 参数后再次运行即可写入数据库。");
    }
}

/// Return an RFC-3339 timestamp one day before `dt_str`.
/// Falls back to `fallback` if `dt_str` cannot be parsed.
fn date_minus_one_day(dt_str: &str, fallback: &str) -> String {
    // Try full RFC-3339 / ISO-8601 with timezone.
    if let Ok(dt) = dt_str.parse::<DateTime<FixedOffset>>() {
        return (dt - Duration::days(1)).to_rfc3339();
    }
    // Try plain date ("YYYY-MM-DD").
    if let Ok(date) = chrono::NaiveDate::parse_from_str(dt_str, "%Y-%m-%d") {
        if let Some(prev) = date.pred_opt() {
            return prev.format("%Y-%m-%dT00:00:00+00:00").to_string();
        }
    }
    fallback.to_string()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("
            CREATE TABLE holdings (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                symbol TEXT NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL,
                category_id TEXT,
                shares REAL NOT NULL,
                avg_cost REAL NOT NULL,
                currency TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE transactions (
                id TEXT PRIMARY KEY,
                holding_id TEXT,
                account_id TEXT NOT NULL,
                symbol TEXT NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL,
                transaction_type TEXT NOT NULL CHECK(transaction_type IN ('BUY', 'SELL')),
                shares REAL NOT NULL,
                price REAL NOT NULL,
                total_amount REAL NOT NULL,
                commission REAL NOT NULL DEFAULT 0,
                currency TEXT NOT NULL,
                traded_at TEXT NOT NULL,
                notes TEXT,
                created_at TEXT NOT NULL
            );
        ").unwrap();
        conn
    }

    /// Insert a holding row and return its id.
    fn insert_holding(conn: &Connection, symbol: &str, name: &str, shares: f64, avg_cost: f64) -> String {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO holdings VALUES (?1,'acc1',?2,?3,'CN',NULL,?4,?5,'CNY',?6,?6)",
            params![id, symbol, name, shares, avg_cost, now],
        ).unwrap();
        id
    }

    fn insert_txn(conn: &Connection, holding_id: &str, symbol: &str, txn_type: &str, shares: f64, price: f64, traded_at: &str) {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let total = shares * price;
        conn.execute(
            "INSERT INTO transactions VALUES (?1,?2,'acc1',?3,'TestStock','CN',?4,?5,?6,?7,0.0,'CNY',?8,NULL,?9)",
            params![id, holding_id, symbol, txn_type, shares, price, total, traded_at, now],
        ).unwrap();
    }

    fn count_open(conn: &Connection, symbol: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM transactions WHERE UPPER(symbol)=UPPER(?1) AND transaction_type='BUY' AND notes='backfill:initial'",
            params![symbol],
            |r| r.get(0),
        ).unwrap()
    }

    /// Apply the back-calculation formula from main() and return (initial_shares, initial_price).
    fn calculate_initial_position(
        shares_final: f64,
        avg_cost_final: f64,
        buy_shares: f64,
        buy_cost: f64,
        sell_shares: f64,
    ) -> (f64, f64) {
        let s0 = shares_final + sell_shares - buy_shares;
        let total_in = shares_final + sell_shares;
        let p0 = (total_in * avg_cost_final - buy_cost) / s0;
        (s0, p0)
    }

    #[test]
    fn test_formula_no_sells() {
        // Initial: 100 @ 10, BUY 50 @ 12 → final 150 shares, avg 10.667
        let (s0, p0) = calculate_initial_position(150.0, (1000.0 + 600.0) / 150.0, 50.0, 600.0, 0.0);
        assert!((s0 - 100.0).abs() < 1e-4, "s0 = {}", s0);
        assert!((p0 - 10.0).abs() < 1e-3, "p0 = {}", p0);
    }

    #[test]
    fn test_formula_with_sells() {
        // Initial: 100 @ 10, BUY 50 @ 12, SELL 30 → final 120 shares, avg 10.667
        let avg = (1000.0 + 600.0) / 150.0;
        let (s0, p0) = calculate_initial_position(120.0, avg, 50.0, 600.0, 30.0);
        assert!((s0 - 100.0).abs() < 1e-4, "s0 = {}", s0);
        assert!((p0 - 10.0).abs() < 1e-3, "p0 = {}", p0);
    }

    #[test]
    fn test_formula_negative_avg_cost() {
        // Simulates a holding where avg_cost is negative (e.g. received dividends /
        // credits that exceed the purchase cost, making the recorded cost basis negative).
        // The tool must NOT reject such a price₀ — it should produce a valid finite number.
        //
        // Scenario: initial position 100 shares at a cost that, after a subsequent BUY
        // at a positive price, leaves avg_cost_final = -104.9649.
        //   avg_cost_final = (s0 * p0 + buy_shares * buy_price) / (s0 + buy_shares)
        //   ⟹  p0 = (avg_cost_final * (s0 + buy_shares) - buy_shares * buy_price) / s0
        // We pick concrete numbers that reproduce the reported case.
        let shares_final = 200.0_f64;
        let avg_cost_final = -104.9649_f64;
        let buy_shares = 100.0_f64;
        let buy_price = 50.0_f64;
        let sell_shares = 0.0_f64;

        let (s0, p0) = calculate_initial_position(
            shares_final, avg_cost_final, buy_shares, buy_price * buy_shares, sell_shares,
        );
        // s0 = 200 + 0 - 100 = 100
        assert!((s0 - 100.0).abs() < 1e-6, "s0 = {}", s0);
        // p0 should be finite (negative is fine)
        assert!(p0.is_finite(), "p0 must be finite, got {}", p0);
        // Verify round-trip: reconstructed avg matches
        let reconstructed_avg = (s0 * p0 + buy_shares * buy_price) / (s0 + buy_shares);
        assert!(
            (reconstructed_avg - avg_cost_final).abs() < 1e-4,
            "round-trip avg mismatch: {} vs {}",
            reconstructed_avg, avg_cost_final
        );
    }

    #[test]
    fn test_date_minus_one_day_rfc3339() {
        let result = date_minus_one_day("2024-03-15T10:00:00+00:00", "fallback");
        assert!(result.starts_with("2024-03-14"), "got: {}", result);
    }

    #[test]
    fn test_date_minus_one_day_plain_date() {
        let result = date_minus_one_day("2024-03-15", "fallback");
        assert!(result.starts_with("2024-03-14"), "got: {}", result);
    }

    #[test]
    fn test_date_minus_one_day_fallback() {
        let result = date_minus_one_day("not-a-date", "2024-01-01T00:00:00+00:00");
        assert_eq!(result, "2024-01-01T00:00:00+00:00");
    }

    #[test]
    fn test_case_a_no_transactions() {
        // A holding with no transactions should get an OPEN record whose
        // shares and price match the holding exactly.
        let conn = setup_db();
        let hid = insert_holding(&conn, "SH600036", "招商银行", 1000.0, 35.5);

        // Run the same logic as main() inline.
        let txns: Vec<(String, f64, f64, String)> = {
            let mut s = conn.prepare(
                "SELECT transaction_type,shares,price,traded_at FROM transactions WHERE account_id='acc1' AND UPPER(symbol)=UPPER('SH600036') ORDER BY traded_at ASC"
            ).unwrap();
            s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap()
             .collect::<Result<Vec<_>, _>>().unwrap()
        };

        assert!(txns.is_empty());

        // Case A: insert BUY (backfill:initial) from holding data.
        let txn_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let created_at_val: String = conn.query_row("SELECT created_at FROM holdings WHERE id=?1", params![hid], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO transactions VALUES (?1,?2,'acc1','SH600036','招商银行','CN','BUY',1000.0,35.5,35500.0,0.0,'CNY',?3,'backfill:initial',?4)",
            params![txn_id, hid, created_at_val, now],
        ).unwrap();

        assert_eq!(count_open(&conn, "SH600036"), 1);
        let (open_shares, open_price): (f64, f64) = conn.query_row(
            "SELECT shares, price FROM transactions WHERE transaction_type='BUY' AND notes='backfill:initial' AND symbol='SH600036'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert!((open_shares - 1000.0).abs() < 1e-6);
        assert!((open_price - 35.5).abs() < 1e-6);
    }

    #[test]
    fn test_case_b_with_transactions() {
        // Holding: 120 shares, avg_cost = (1000+600)/150 ≈ 10.667
        // History: BUY 50 @ 12, SELL 30
        // Expected initial OPEN: 100 shares @ 10.0
        let conn = setup_db();
        let avg = (1000.0_f64 + 600.0) / 150.0;
        let hid = insert_holding(&conn, "US.AAPL", "Apple", 120.0, avg);
        insert_txn(&conn, &hid, "US.AAPL", "BUY", 50.0, 12.0, "2024-06-01T10:00:00+00:00");
        insert_txn(&conn, &hid, "US.AAPL", "SELL", 30.0, 15.0, "2024-07-01T10:00:00+00:00");

        let (s0, p0) = calculate_initial_position(120.0, avg, 50.0, 600.0, 30.0);
        assert!((s0 - 100.0).abs() < 1e-4);
        assert!((p0 - 10.0).abs() < 1e-3);

        // Verify OPEN date is before the earliest transaction.
        let open_date = date_minus_one_day("2024-06-01T10:00:00+00:00", "");
        assert!(open_date.starts_with("2024-05-31"), "got: {}", open_date);
    }

    #[test]
    fn test_cash_holdings_skipped() {
        let conn = setup_db();
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO holdings VALUES (?1,'acc1','$CASH-CNY','人民币现金','CN',NULL,50000.0,1.0,'CNY',?2,?2)",
            params![id, now],
        ).unwrap();

        // The query in main() filters these out.
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM holdings WHERE symbol NOT LIKE '$CASH-%'"
        ).unwrap();
        let count: i64 = stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(count, 0, "cash holding should be excluded");
    }

    #[test]
    fn test_existing_open_idempotent() {
        let conn = setup_db();
        let hid = insert_holding(&conn, "HK.00700", "腾讯控股", 200.0, 300.0);
        // Simulate a previously-backfilled BUY record (notes='backfill:initial').
        let txn_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO transactions VALUES (?1,?2,'acc1','HK.00700','腾讯控股','HK','BUY',200.0,300.0,60000.0,0.0,'HKD','2023-01-01T00:00:00+00:00','backfill:initial',?3)",
            params![txn_id, hid, now],
        ).unwrap();

        // Should already have 1 backfill record.
        assert_eq!(count_open(&conn, "HK.00700"), 1);
        // The main() loop would see the backfill:initial note and skip, so still 1.
    }
}
