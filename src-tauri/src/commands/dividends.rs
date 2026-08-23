use crate::db::Database;
use crate::models::dividend::{
    AccountDividend, CurrencyDividend, DividendAnalysis, DividendEntry, DividendRow,
};
use tauri::State;
use tracing::warn;

/// Aggregate PAY (dividend) transactions for one year into per-currency tables:
/// row = company, column = account, plus totals. Amounts are net of
/// commission (total_amount - commission) and stay in each transaction's actual
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
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Group key: (currency, symbol) -> ordered accounts, per-account totals.
    // We keep account order as encountered (sorted by account name later for
    // stable columns).
    #[derive(Default)]
    struct CompanyAcc {
        name: String,
        accounts: std::collections::BTreeMap<String, f64>, // account_id -> total
    }
    let mut by_currency: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, CompanyAcc>,
    > = std::collections::BTreeMap::new();
    // account_id -> name
    let mut account_info: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();

    {
        let mut stmt = conn
            .prepare(
                "SELECT t.currency, t.symbol, t.name, t.account_id, a.name,
                        SUM(t.total_amount - t.commission)
                 FROM transactions t
                 JOIN accounts a ON t.account_id = a.id
                 WHERE t.transaction_type = 'PAY'
                   AND (?1 IS NULL OR strftime('%Y', t.traded_at) = ?1)
                 GROUP BY t.currency, t.symbol, t.name, t.account_id, a.name
                 ORDER BY t.currency, SUM(t.total_amount - t.commission) DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                rusqlite::params![year.map(|value| value.to_string())],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?, // currency
                        row.get::<_, String>(1)?, // symbol
                        row.get::<_, String>(2)?, // name
                        row.get::<_, String>(3)?, // account_id
                        row.get::<_, String>(4)?, // account name
                        row.get::<_, f64>(5)?,    // total
                    ))
                },
            )
            .map_err(|e| e.to_string())?;

        for r in rows {
            let (currency, symbol, name, account_id, account_name, total) =
                r.map_err(|e| e.to_string())?;
            account_info.insert(account_id.clone(), account_name);
            let company = by_currency
                .entry(currency)
                .or_default()
                .entry(symbol)
                .or_insert_with(|| CompanyAcc {
                    name,
                    accounts: Default::default(),
                });
            *company.accounts.entry(account_id).or_insert(0.0) += total;
        }
    }

    // Preserve the month/account/market dimensions needed by the alternative
    // frontend summaries. Multiple PAY rows in the same month are collapsed.
    let entries: Vec<DividendEntry> = {
        let mut stmt = conn
            .prepare(
                "SELECT t.account_id, a.name, a.market, t.symbol, t.name, t.market,
                        t.currency, strftime('%Y%m', t.traded_at),
                        SUM(t.total_amount - t.commission)
                 FROM transactions t
                 JOIN accounts a ON t.account_id = a.id
                 WHERE t.transaction_type = 'PAY'
                   AND (?1 IS NULL OR strftime('%Y', t.traded_at) = ?1)
                 GROUP BY t.account_id, a.name, a.market, t.symbol, t.name,
                          t.market, t.currency, strftime('%Y%m', t.traded_at)
                 ORDER BY a.name, t.market, t.symbol,
                          strftime('%Y%m', t.traded_at) DESC, t.currency",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                rusqlite::params![year.map(|value| value.to_string())],
                |row| {
                    Ok(DividendEntry {
                        account_id: row.get(0)?,
                        account_name: row.get(1)?,
                        account_market: row.get(2)?,
                        symbol: row.get(3)?,
                        name: row.get(4)?,
                        market: row.get(5)?,
                        currency: row.get(6)?,
                        month: row.get(7)?,
                        total: row.get(8)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|e: rusqlite::Error| e.to_string())?
    };

    // Build the per-currency structures. Accounts = all accounts that have
    // dividends in this currency, sorted by name; rows sorted by total descending.
    let mut currencies: Vec<CurrencyDividend> = Vec::new();
    let mut grand_total = 0.0f64;
    for (currency, companies) in by_currency {
        // Collect distinct account ids for this currency, sorted by account name.
        let acct_ids: Vec<String> = {
            let mut v: Vec<String> = companies
                .values()
                .flat_map(|c| c.accounts.keys().cloned())
                .collect();
            v.sort_by(|a, b| account_info[a].cmp(&account_info[b]));
            v.dedup();
            v
        };
        // Account totals across all companies.
        let mut acct_totals: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        let accounts: Vec<AccountDividend> = acct_ids
            .iter()
            .map(|id| {
                let name = &account_info[id];
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
        currencies.push(CurrencyDividend {
            currency,
            accounts,
            rows,
            total,
        });
    }

    // Sort currency groups in a stable, sensible order.
    let order = |currency: &str| match currency {
        "CNY" => 0,
        "USD" => 1,
        "HKD" => 2,
        _ => 3,
    };
    currencies.sort_by_key(|group| order(&group.currency));

    if currencies.is_empty() {
        match year {
            Some(value) => warn!("[分红分析] {} 年无分红记录", value),
            None => warn!("[分红分析] 无分红记录"),
        }
    }

    Ok(DividendAnalysis {
        year,
        currencies,
        entries,
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

    /// Build an in-memory DB with CNY and USD dividends. One HK-market stock
    /// is held through Stock Connect and pays into the CN account in CNY.
    fn db_with_dividends() -> Database {
        let db = Database::new(":memory:").expect("failed to create in-memory database");
        let ts = now();
        let insert = |conn: &rusqlite::Connection,
                      id: &str,
                      account_id: &str,
                      symbol: &str,
                      name: &str,
                      market: &str,
                      currency: &str,
                      amount: f64,
                      comm: f64,
                      traded: &str| {
            conn.execute(
                "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market,
                        transaction_type, shares, price, total_amount, commission, currency,
                        traded_at, notes, created_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, 'PAY', 0.0, 0.0, ?7, ?8, ?6, ?9, NULL, ?10)",
                rusqlite::params![
                    id, account_id, symbol, name, market, currency, amount, comm, traded, ts
                ],
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
                "CNY",
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
                "CNY",
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
                "CNY",
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
                "USD",
                100.0,
                1.0,
                "2025-02-14",
            ); // net 99
            insert(
                &conn,
                "t6",
                "a1",
                "00883",
                "中国海洋石油",
                "HK",
                "CNY",
                4_444.78,
                0.0,
                "2025-06-30",
            );
            // A 2024 dividend that must be excluded when filtering year 2025.
            insert(
                &conn,
                "t5",
                "a1",
                "000333.SZ",
                "美的集团",
                "CN",
                "CNY",
                999.0,
                0.0,
                "2024-06-10",
            );
        }
        db
    }

    #[test]
    fn test_dividend_analysis_aggregates_by_actual_currency_and_account() {
        let db = db_with_dividends();
        let analysis =
            get_dividend_analysis_inner(&db, Some(2025)).expect("analysis should succeed");
        assert_eq!(analysis.year, Some(2025));

        // Two actual currencies are present. The HK-market Stock Connect
        // dividend belongs to CNY rather than creating an HKD group.
        assert_eq!(analysis.currencies.len(), 2);
        let cny = analysis
            .currencies
            .iter()
            .find(|g| g.currency == "CNY")
            .unwrap();
        let usd = analysis
            .currencies
            .iter()
            .find(|g| g.currency == "USD")
            .unwrap();
        assert!(analysis.currencies.iter().all(|g| g.currency != "HKD"));

        // CNY account columns sorted by account name: 中信证券, 平安证券.
        assert_eq!(cny.accounts.len(), 2);
        assert_eq!(cny.accounts[0].account_name, "中信证券");
        assert_eq!(cny.accounts[1].account_name, "平安证券");
        assert!(
            (cny.total - 212_161.13).abs() < 0.01,
            "CNY total {}",
            cny.total
        );
        assert_eq!(cny.rows.len(), 3);
        assert_eq!(cny.rows[0].symbol, "000333.SZ");
        assert!((cny.rows[0].total - 165_680.0).abs() < 0.01);
        let cnooc = cny.rows.iter().find(|row| row.symbol == "00883").unwrap();
        assert!((cnooc.total - 4_444.78).abs() < 0.01);
        let cnooc_entry = analysis
            .entries
            .iter()
            .find(|entry| entry.symbol == "00883")
            .unwrap();
        assert_eq!(cnooc_entry.account_market, "CN");
        assert_eq!(cnooc_entry.market, "HK");
        assert_eq!(cnooc_entry.currency, "CNY");
        assert_eq!(cnooc_entry.month, "202506");
        assert!((cnooc_entry.total - 4_444.78).abs() < 0.01);
        // 美的集团 per-account: 中信 57000, 平安 108680
        let midea = &cny.rows[0];
        assert!((midea.per_account[0].1 - 57_000.0).abs() < 0.01);
        assert!((midea.per_account[1].1 - 108_680.0).abs() < 0.01);

        // US: net = total - commission = 99
        assert!((usd.total - 99.0).abs() < 0.01);
        assert_eq!(usd.rows[0].symbol, "AAPL");
        assert!((usd.rows[0].per_account[0].1 - 99.0).abs() < 0.01);

        // Grand total remains a raw cross-currency sum; frontend converts it.
        assert!((analysis.grand_total - (212_161.13 + 99.0)).abs() < 0.01);
    }

    #[test]
    fn test_dividend_analysis_year_filter_excludes_other_years() {
        let db = db_with_dividends();
        let analysis =
            get_dividend_analysis_inner(&db, Some(2024)).expect("analysis should succeed");
        // Only the 2024 美的集团 999 dividend.
        assert_eq!(analysis.currencies.len(), 1);
        assert_eq!(analysis.currencies[0].currency, "CNY");
        assert_eq!(analysis.currencies[0].rows.len(), 1);
        assert!((analysis.currencies[0].rows[0].total - 999.0).abs() < 0.01);
    }

    #[test]
    fn test_dividend_analysis_without_year_includes_all_history() {
        let db = db_with_dividends();
        let analysis = get_dividend_analysis_inner(&db, None).expect("analysis should succeed");
        assert_eq!(analysis.year, None);

        let cny = analysis
            .currencies
            .iter()
            .find(|group| group.currency == "CNY")
            .unwrap();
        assert!((cny.total - 213_160.13).abs() < 0.01);
        assert!((analysis.grand_total - (213_160.13 + 99.0)).abs() < 0.01);
    }

    #[test]
    fn test_dividend_analysis_no_data_year() {
        let db = db_with_dividends();
        let analysis =
            get_dividend_analysis_inner(&db, Some(2030)).expect("analysis should succeed");
        assert!(analysis.currencies.is_empty());
        assert!(analysis.entries.is_empty());
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
