use crate::db::Database;
use crate::models::dividend::{AccountDividend, DividendAnalysis, DividendRow, MarketDividend};
use chrono::Datelike;
use tauri::State;
use tracing::warn;

/// Currency of each market (native).
fn market_currency(market: &str) -> &'static str {
    match market {
        "CN" => "CNY",
        "US" => "USD",
        "HK" => "HKD",
        _ => "USD",
    }
}

/// Aggregate PAY (dividend) transactions for one year into per-market tables:
/// row = company, column = account, plus totals. Amounts are net of
/// commission (total_amount - commission) and stay in the market's native
/// currency — the frontend converts to a chosen base currency for the grand
/// total using its exchange-rate store.
#[tauri::command(rename_all = "camelCase")]
pub fn get_dividend_analysis(
    db: State<Database>,
    year: Option<i32>,
) -> Result<DividendAnalysis, String> {
    get_dividend_analysis_inner(&db, year)
}

fn get_dividend_analysis_inner(
    db: &Database,
    year: Option<i32>,
) -> Result<DividendAnalysis, String> {
    let year = year.unwrap_or_else(|| chrono::Local::now().year());
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Group key: (market, symbol) -> ordered accounts, per-account totals.
    // We keep account order as encountered (sorted by account name later for
    // stable columns).
    #[derive(Default)]
    struct CompanyAcc {
        name: String,
        accounts: std::collections::BTreeMap<String, f64>, // account_id -> total
    }
    let mut by_market: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, CompanyAcc>,
    > = std::collections::BTreeMap::new();
    // account_id -> (name, market)
    let mut account_info: std::collections::BTreeMap<String, (String, String)> =
        std::collections::BTreeMap::new();

    {
        let mut stmt = conn
            .prepare(
                "SELECT t.market, t.symbol, t.name, t.account_id, a.name,
                        SUM(t.total_amount - t.commission)
                 FROM transactions t
                 JOIN accounts a ON t.account_id = a.id
                 WHERE t.transaction_type = 'PAY'
                   AND strftime('%Y', t.traded_at) = ?1
                 GROUP BY t.market, t.symbol, t.name, t.account_id, a.name
                 ORDER BY t.market, SUM(t.total_amount - t.commission) DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![year.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?, // market
                    row.get::<_, String>(1)?, // symbol
                    row.get::<_, String>(2)?, // name
                    row.get::<_, String>(3)?, // account_id
                    row.get::<_, String>(4)?, // account name
                    row.get::<_, f64>(5)?,    // total
                ))
            })
            .map_err(|e| e.to_string())?;

        for r in rows {
            let (market, symbol, name, account_id, account_name, total) =
                r.map_err(|e| e.to_string())?;
            account_info.insert(account_id.clone(), (account_name, market.clone()));
            let company = by_market
                .entry(market)
                .or_default()
                .entry(symbol)
                .or_insert_with(|| CompanyAcc {
                    name,
                    accounts: Default::default(),
                });
            *company.accounts.entry(account_id).or_insert(0.0) += total;
        }
    }

    // Build the per-market structures. Accounts = all accounts that appear in
    // this market, sorted by name; rows sorted by company total descending.
    let mut markets: Vec<MarketDividend> = Vec::new();
    let mut grand_total = 0.0f64;
    for (market, companies) in by_market {
        // Collect distinct account ids for this market, sorted by account name.
        let acct_ids: Vec<String> = {
            let mut v: Vec<String> = companies
                .values()
                .flat_map(|c| c.accounts.keys().cloned())
                .collect();
            v.sort_by(|a, b| {
                let (an, _) = &account_info[a];
                let (bn, _) = &account_info[b];
                an.cmp(bn)
            });
            v.dedup();
            v
        };
        // Account totals across all companies.
        let mut acct_totals: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        let accounts: Vec<AccountDividend> = acct_ids
            .iter()
            .map(|id| {
                let (name, _) = &account_info[id];
                let total = companies
                    .values()
                    .map(|c| c.accounts.get(id).copied().unwrap_or(0.0))
                    .sum();
                acct_totals.insert(id.clone(), total);
                AccountDividend {
                    account_id: id.clone(),
                    account_name: name.clone(),
                    total,
                }
            })
            .collect();

        // Rows: company, per-account amounts, company total.
        let mut rows: Vec<DividendRow> = companies
            .into_iter()
            .map(|(symbol, c)| {
                let per_account: Vec<(String, f64)> = acct_ids
                    .iter()
                    .map(|id| (id.clone(), c.accounts.get(id).copied().unwrap_or(0.0)))
                    .collect();
                let total = c.accounts.values().sum::<f64>();
                DividendRow {
                    symbol,
                    name: c.name,
                    per_account,
                    total,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.total
                .partial_cmp(&a.total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total: f64 = rows.iter().map(|r| r.total).sum();
        grand_total += total;
        let currency = market_currency(&market).to_string();
        markets.push(MarketDividend {
            market,
            currency,
            accounts,
            rows,
            total,
        });
    }

    // Sort markets in a stable, sensible order: CN, US, HK.
    let order = |m: &str| match m {
        "CN" => 0,
        "US" => 1,
        "HK" => 2,
        _ => 3,
    };
    markets.sort_by_key(|m| order(&m.market));

    if markets.is_empty() {
        warn!("[分红分析] {} 年无分红记录", year);
    }

    Ok(DividendAnalysis {
        year,
        markets,
        grand_total,
    })
}

/// Distinct years that have PAY (dividend) transactions, newest first.
/// Used to populate the year selector.
#[tauri::command(rename_all = "camelCase")]
pub fn get_dividend_years(db: State<Database>) -> Result<Vec<i32>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT CAST(strftime('%Y', traded_at) AS INTEGER) AS y
             FROM transactions
             WHERE transaction_type = 'PAY'
             ORDER BY y DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, i32>(0))
        .map_err(|e| e.to_string())?;
    let mut years = Vec::new();
    for r in rows {
        years.push(r.map_err(|e| e.to_string())?);
    }
    Ok(years)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    /// Build an in-memory DB with two CN accounts and one US account, plus
    /// PAY dividends in 2025 for a couple of companies.
    fn db_with_dividends() -> Database {
        let db = Database::new(":memory:").expect("failed to create in-memory database");
        let ts = now();
        let insert = |conn: &rusqlite::Connection,
                      id: &str,
                      account_id: &str,
                      symbol: &str,
                      name: &str,
                      market: &str,
                      amount: f64,
                      comm: f64,
                      traded: &str| {
            conn.execute(
                "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market,
                        transaction_type, shares, price, total_amount, commission, currency,
                        traded_at, notes, created_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, 'PAY', 0.0, 0.0, ?6, ?7, 'CNY', ?8, NULL, ?9)",
                rusqlite::params![id, account_id, symbol, name, market, amount, comm, traded, ts],
            )
            .unwrap();
        };
        {
            let conn = db.conn.lock().unwrap();
            // Accounts
            for (id, name, market) in [
                ("a1", "平安证券", "CN"),
                ("a2", "中信证券", "CN"),
                ("a3", "US Broker", "US"),
            ] {
                conn.execute(
                    "INSERT INTO accounts (id, name, market, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    rusqlite::params![id, name, market, ts],
                )
                .unwrap();
            }
            // CN 2025: 美的集团 across two accounts; 贵州茅台 one account.
            insert(
                &conn,
                "t1",
                "a1",
                "000333.SZ",
                "美的集团",
                "CN",
                108_680.0,
                0.0,
                "2025-06-10",
            );
            insert(
                &conn,
                "t2",
                "a2",
                "000333.SZ",
                "美的集团",
                "CN",
                57_000.0,
                0.0,
                "2025-06-10",
            );
            insert(
                &conn,
                "t3",
                "a1",
                "600519.SH",
                "贵州茅台",
                "CN",
                42_036.35,
                0.0,
                "2025-07-01",
            );
            insert(
                &conn,
                "t4",
                "a3",
                "AAPL",
                "Apple",
                "US",
                100.0,
                1.0,
                "2025-02-14",
            ); // net 99
               // A 2024 dividend that must be excluded when filtering year 2025.
            insert(
                &conn,
                "t5",
                "a1",
                "000333.SZ",
                "美的集团",
                "CN",
                999.0,
                0.0,
                "2024-06-10",
            );
        }
        db
    }

    #[test]
    fn test_dividend_analysis_aggregates_by_market_and_account() {
        let db = db_with_dividends();
        let analysis =
            get_dividend_analysis_inner(&db, Some(2025)).expect("analysis should succeed");
        assert_eq!(analysis.year, 2025);

        // Two markets present: CN and US (HK has none).
        assert_eq!(analysis.markets.len(), 2);
        let cn = analysis.markets.iter().find(|m| m.market == "CN").unwrap();
        let us = analysis.markets.iter().find(|m| m.market == "US").unwrap();

        // CN currency, account columns sorted by name: 平安证券, 中信证券
        assert_eq!(cn.currency, "CNY");
        assert_eq!(cn.accounts.len(), 2);
        assert_eq!(cn.accounts[0].account_name, "中信证券");
        assert_eq!(cn.accounts[1].account_name, "平安证券");
        // CN total = 108680 + 57000 + 42036.35 = 207716.35
        assert!(
            (cn.total - 207_716.35).abs() < 0.01,
            "CN total {}",
            cn.total
        );
        // CN rows: 美的集团 (total 165680), 贵州茅台 (42036.35)
        assert_eq!(cn.rows.len(), 2);
        assert_eq!(cn.rows[0].symbol, "000333.SZ");
        assert!((cn.rows[0].total - 165_680.0).abs() < 0.01);
        // 美的集团 per-account: 中信 57000, 平安 108680
        let midea = &cn.rows[0];
        assert!((midea.per_account[0].1 - 57_000.0).abs() < 0.01);
        assert!((midea.per_account[1].1 - 108_680.0).abs() < 0.01);

        // US: net = total - commission = 99
        assert_eq!(us.currency, "USD");
        assert!((us.total - 99.0).abs() < 0.01);
        assert_eq!(us.rows[0].symbol, "AAPL");
        assert!((us.rows[0].per_account[0].1 - 99.0).abs() < 0.01);

        // Grand total = CN + US native sums (not converted).
        assert!((analysis.grand_total - (207_716.35 + 99.0)).abs() < 0.01);
    }

    #[test]
    fn test_dividend_analysis_year_filter_excludes_other_years() {
        let db = db_with_dividends();
        let analysis =
            get_dividend_analysis_inner(&db, Some(2024)).expect("analysis should succeed");
        // Only the 2024 美的集团 999 dividend.
        assert_eq!(analysis.markets.len(), 1);
        assert_eq!(analysis.markets[0].market, "CN");
        assert_eq!(analysis.markets[0].rows.len(), 1);
        assert!((analysis.markets[0].rows[0].total - 999.0).abs() < 0.01);
    }

    #[test]
    fn test_dividend_analysis_no_data_year() {
        let db = db_with_dividends();
        let analysis =
            get_dividend_analysis_inner(&db, Some(2030)).expect("analysis should succeed");
        assert!(analysis.markets.is_empty());
        assert_eq!(analysis.grand_total, 0.0);
    }

    #[test]
    fn test_dividend_years_returns_distinct_years_desc() {
        let db = db_with_dividends();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT CAST(strftime('%Y', traded_at) AS INTEGER) AS y
                 FROM transactions WHERE transaction_type = 'PAY' ORDER BY y DESC",
            )
            .unwrap();
        let years: Vec<i32> = stmt
            .query_map([], |row| row.get::<_, i32>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(years, vec![2025, 2024]);
    }
}
