#[rustfmt::skip]
use super::*;
use super::calculation::{
    calculate_max_drawdown, fetch_previous_day_value, parse_required_exchange_rates,
    performance_load_count, reset_performance_load_count,
};
use crate::models::performance::{annualise_return, parse_date, ReturnDataPoint};

#[test]
fn previous_value_lookup_distinguishes_missing_from_bad_dates() {
    let db = Database::new(":memory:").unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    assert!(
        fetch_previous_day_value(&db, cutoff, &PerformanceFilter::default())
            .unwrap()
            .is_none()
    );

    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO daily_portfolio_values (date, total_value)
             VALUES ('2025-bad-date', 42)",
        [],
    )
    .unwrap();
    drop(conn);

    let error = fetch_previous_day_value(&db, cutoff, &PerformanceFilter::default()).unwrap_err();
    assert!(error.contains("bad date '2025-bad-date'"));
}

#[test]
fn filtered_previous_value_lookup_propagates_bad_dates() {
    let db = Database::new(":memory:").unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO daily_holding_snapshots
             (date, account_id, symbol, market, shares, avg_cost, close_price, market_value)
             VALUES ('2025-bad-date', 'acct', 'AAPL', 'US', 1, 1, 42, 42)",
        [],
    )
    .unwrap();
    drop(conn);
    let filter = PerformanceFilter {
        market: Some("US".to_string()),
        account_id: None,
    };

    let error =
        fetch_previous_day_value(&db, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), &filter)
            .unwrap_err();

    assert!(error.contains("bad date '2025-bad-date'"));
}

#[test]
fn required_rate_parser_distinguishes_missing_from_malformed_json() {
    let missing = parse_required_exchange_rates(None, "cash flow").unwrap_err();
    assert!(missing.contains("missing exchange rates for cash flow"));

    let malformed = parse_required_exchange_rates(Some("not-json"), "cash flow").unwrap_err();
    assert!(malformed.contains("invalid exchange rates for cash flow"));
}

fn cash_flow_performance_db() -> Database {
    let db = Database::new(":memory:").unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-us', 'US account', 'US', NULL, '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();
        for (date, value) in [
            ("2024-01-01", 100.0),
            ("2024-01-02", 110.0),
            ("2024-01-03", 165.0),
        ] {
            conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, ?2, 0, ?2, 0, 0, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date, value],
                )
                .unwrap();
        }
        conn.execute(
            "INSERT INTO transactions
                 (id, holding_id, account_id, symbol, name, market, transaction_type,
                  shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES ('cash-in', NULL, 'acct-us', '$CASH-USD', 'Cash', 'US', 'BUY',
                         0, 0, 50, 0, 'USD', '2024-01-03T09:00:00Z', NULL, '2024-01-03')",
            [],
        )
        .unwrap();
    }
    db
}

fn sold_holding_performance_db() -> Database {
    let db = Database::new(":memory:").unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-us', 'US account', 'US', NULL, '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('holding-aapl', 'acct-us', 'AAPL', 'Apple', 'US', NULL, 0, 100,
                         'USD', '2024-01-01', '2024-01-03')",
            [],
        )
        .unwrap();
        for (date, value) in [
            ("2024-01-01", 100.0),
            ("2024-01-02", 104.0),
            ("2024-01-03", 122.0),
        ] {
            conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, ?2, 0, ?2, 0, 0, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date, value],
                )
                .unwrap();
        }
        conn.execute(
            "INSERT INTO daily_holding_snapshots
                 (date, account_id, symbol, market, category_name, shares, avg_cost,
                  close_price, market_value)
                 VALUES ('2024-01-01', 'acct-us', 'AAPL', 'US', '成长股', 1, 100, 100, 100)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO daily_holding_snapshots
                 (date, account_id, symbol, market, category_name, shares, avg_cost,
                  close_price, market_value)
                 VALUES ('2024-01-03', 'acct-us', '$CASH-USD', 'US', NULL, 122, 1, 1, 122)",
            [],
        )
        .unwrap();
        for (id, transaction_type, total_amount, commission) in [
            ("aapl-dividend", "PAY", 5.0, 1.0),
            ("aapl-sell", "SELL", 120.0, 2.0),
        ] {
            conn.execute(
                    "INSERT INTO transactions
                     (id, holding_id, account_id, symbol, name, market, transaction_type,
                      shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                     VALUES (?1, 'holding-aapl', 'acct-us', 'AAPL', 'Apple', 'US', ?2,
                             1, 120, ?3, ?4, 'USD', '2024-01-02T09:00:00Z', NULL, '2024-01-02')",
                    rusqlite::params![id, transaction_type, total_amount, commission],
                )
                .unwrap();
        }
    }
    db
}

fn roundtrip_holding_performance_db() -> Database {
    let db = sold_holding_performance_db();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('holding-msft', 'acct-us', 'MSFT', 'Microsoft', 'US', NULL, 0, 100,
                         'USD', '2024-01-02', '2024-01-02')",
            [],
        )
        .unwrap();
        for (id, transaction_type, total_amount, commission) in [
            ("msft-buy", "BUY", 100.0, 1.0),
            ("msft-sell", "SELL", 120.0, 2.0),
        ] {
            conn.execute(
                    "INSERT INTO transactions
                     (id, holding_id, account_id, symbol, name, market, transaction_type,
                      shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                     VALUES (?1, 'holding-msft', 'acct-us', 'MSFT', 'Microsoft', 'US', ?2,
                             1, 100, ?3, ?4, 'USD', '2024-01-02T10:00:00Z', NULL, '2024-01-02')",
                    rusqlite::params![id, transaction_type, total_amount, commission],
                )
                .unwrap();
        }
    }
    db
}

fn multicurrency_attribution_db() -> Database {
    let db = Database::new(":memory:").unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-cn', 'CN account', 'CN', NULL, '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('holding-cn', 'acct-cn', '600000', '浦发银行', 'CN', NULL, 100, 7.2,
                         'CNY', '2024-01-01', '2024-01-03')",
            [],
        )
        .unwrap();
        for (date, native_value, usd_value) in [
            ("2024-01-01", 720.0, 100.0),
            ("2024-01-02", 756.0, 105.0),
            ("2024-01-03", 792.0, 110.0),
        ] {
            conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, ?3, 0, 0, 0, ?2, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date, native_value, usd_value],
                )
                .unwrap();
            conn.execute(
                "INSERT INTO daily_holding_snapshots
                     (date, account_id, symbol, market, category_name, shares, avg_cost,
                      close_price, market_value)
                     VALUES (?1, 'acct-cn', '600000', 'CN', '分红股', 100, 7.2,
                             ?2 / 100.0, ?2)",
                rusqlite::params![date, native_value],
            )
            .unwrap();
        }
    }
    db
}

fn duplicate_symbol_multicurrency_db() -> Database {
    let db = Database::new(":memory:").unwrap();
    {
        let conn = db.conn.lock().unwrap();
        for (id, name, market) in [
            ("acct-us", "US account", "US"),
            ("acct-cn", "CN account", "CN"),
        ] {
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                     VALUES (?1, ?2, ?3, NULL, '2024-01-01', '2024-01-03')",
                rusqlite::params![id, name, market],
            )
            .unwrap();
        }
        for (id, name) in [("cat-growth", "成长股"), ("cat-value", "价值股")] {
            conn.execute(
                "INSERT INTO categories
                     (id, name, color, icon, is_system, sort_order, created_at)
                     VALUES (?1, ?2, '#000000', '', 0, 0, '2024-01-01')",
                rusqlite::params![id, name],
            )
            .unwrap();
        }
        for (id, account_id, name, market, category_id, currency, shares, avg_cost) in [
            (
                "holding-us",
                "acct-us",
                "Duplicate US",
                "US",
                "cat-growth",
                "USD",
                1.0,
                100.0,
            ),
            (
                "holding-cn",
                "acct-cn",
                "Duplicate CN",
                "CN",
                "cat-value",
                "CNY",
                100.0,
                7.2,
            ),
        ] {
            conn.execute(
                "INSERT INTO holdings
                     (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                      currency, created_at, updated_at)
                     VALUES (?1, ?2, 'DUP', ?3, ?4, ?5, ?7, ?8, ?6,
                             '2024-01-01', '2024-01-03')",
                rusqlite::params![
                    id,
                    account_id,
                    name,
                    market,
                    category_id,
                    currency,
                    shares,
                    avg_cost
                ],
            )
            .unwrap();
        }
        for (date, us_value, cn_value, total_value) in [
            ("2024-01-01", 100.0, 720.0, 200.0),
            ("2024-01-02", 105.0, 756.0, 210.0),
            ("2024-01-03", 110.0, 792.0, 220.0),
        ] {
            conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, ?4, 0, ?2, 0, ?3, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date, us_value, cn_value, total_value],
                )
                .unwrap();
            for (account_id, market, category, shares, avg_cost, close_price, market_value) in [
                ("acct-us", "US", "成长股", 1.0, 100.0, us_value, us_value),
                (
                    "acct-cn",
                    "CN",
                    "价值股",
                    100.0,
                    7.2,
                    cn_value / 100.0,
                    cn_value,
                ),
            ] {
                conn.execute(
                    "INSERT INTO daily_holding_snapshots
                         (date, account_id, symbol, market, category_name, shares, avg_cost,
                          close_price, market_value)
                         VALUES (?1, ?2, 'DUP', ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        date,
                        account_id,
                        market,
                        category,
                        shares,
                        avg_cost,
                        close_price,
                        market_value
                    ],
                )
                .unwrap();
            }
        }
    }
    db
}

fn flat_performance_db() -> Database {
    let db = Database::new(":memory:").unwrap();
    {
        let conn = db.conn.lock().unwrap();
        for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
            conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, 100, 0, 100, 0, 0, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date],
                )
                .unwrap();
        }
    }
    db
}

#[test]
fn test_annualise_return() {
    let ar = annualise_return(0.10, 365);
    assert!((ar - 0.10).abs() < 1e-6);

    let ar2 = annualise_return(0.0, 365);
    assert_eq!(ar2, 0.0);
}

#[test]
fn test_volatility() {
    let returns = vec![1.0, -1.0, 2.0, -2.0, 0.5];
    let (dv, av) = calculate_volatility(&returns);
    assert!(dv > 0.0);
    assert!((av - dv * 252.0_f64.sqrt()).abs() < 1e-9);
}

#[test]
fn test_max_drawdown() {
    let series: Vec<ReturnDataPoint> = vec![
        ReturnDataPoint {
            date: "2024-01-01".to_string(),
            cumulative_return: 0.0,
            daily_return: 0.0,
            portfolio_value: 100.0,
            daily_pnl: 0.0,
        },
        ReturnDataPoint {
            date: "2024-01-02".to_string(),
            cumulative_return: 10.0,
            daily_return: 10.0,
            portfolio_value: 110.0,
            daily_pnl: 10.0,
        },
        ReturnDataPoint {
            date: "2024-01-03".to_string(),
            cumulative_return: -5.0,
            daily_return: -15.0,
            portfolio_value: 95.0,
            daily_pnl: -15.0,
        },
        ReturnDataPoint {
            date: "2024-01-04".to_string(),
            cumulative_return: 5.0,
            daily_return: 10.0,
            portfolio_value: 105.0,
            daily_pnl: 10.0,
        },
    ];
    let dd = calculate_max_drawdown(&series, None);
    // Peak = 110 on day 2, trough = 95 on day 3 → MDD = (95-110)/110 ≈ -13.6%
    assert!(dd.max_drawdown < 0.0);
    assert!((dd.max_drawdown - (-13.636_363_636)).abs() < 0.001);
    assert_eq!(dd.peak_date, "2024-01-02");
    assert_eq!(dd.trough_date, "2024-01-03");
}

#[test]
fn test_twr_series_uses_baseline_and_neutralizes_external_cash_flow() {
    let daily = vec![
        (parse_date("2024-01-02").unwrap(), 110.0, 10.0),
        (parse_date("2024-01-03").unwrap(), 165.0, 55.0),
    ];
    let baseline = Some((parse_date("2024-01-01").unwrap(), 100.0));
    let external_cash_flows = vec![(parse_date("2024-01-03").unwrap(), 50.0)];

    let series = build_twr_return_series(&daily, baseline, &external_cash_flows);

    assert_eq!(series.len(), 2);
    // First visible day must retain the return from the previous valuation.
    assert!((series[0].daily_return - 10.0).abs() < 1e-9);
    // The 50 contribution is not investment performance: (165 - 110 - 50) / 110.
    assert!((series[1].daily_return - 4.545_454_545).abs() < 1e-9);
    // Daily sub-period returns are geometrically linked: 1.10 * 1.0454545 - 1 = 15%.
    assert!((series[1].cumulative_return - 15.0).abs() < 1e-9);
    assert!((series[1].daily_pnl - 5.0).abs() < 1e-9);
}

#[test]
fn test_performance_summary_uses_twr_and_cash_adjusted_pnl() {
    let db = cash_flow_performance_db();

    let summary = get_performance_summary(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    assert!((summary.total_return - 15.0).abs() < 1e-9);
    assert!((summary.total_pnl - 15.0).abs() < 1e-9);
    assert_eq!(summary.start_date, "2024-01-01");
    assert_eq!(summary.end_date, "2024-01-03");
    assert!((summary.return_series[0].daily_return - 10.0).abs() < 1e-9);
    assert!((summary.return_series[1].daily_return - 4.545_454_545).abs() < 1e-9);
}

#[test]
fn test_performance_report_matches_individual_results_and_loads_once() {
    let db = sold_holding_performance_db();
    let start = parse_date("2024-01-02").unwrap();
    let end = parse_date("2024-01-03").unwrap();
    let filter = PerformanceFilter::default();

    let expected_summary = get_performance_summary(&db, start, end, &filter).unwrap();
    let expected_drawdown = get_drawdown_analysis(&db, start, end, &filter).unwrap();
    let expected_attribution = get_return_attribution(&db, start, end, &filter).unwrap();
    let expected_monthly_returns = get_monthly_returns(&db, start, end, &filter).unwrap();
    let expected_holding_performances =
        get_holding_performance_ranking(&db, start, end, "pnl", 10_000, &filter).unwrap();
    let expected_risk_metrics = get_risk_metrics(&db, start, end, &filter).unwrap();

    reset_performance_load_count();
    let report = get_performance_report(&db, start, end, "pnl", 10_000, &filter).unwrap();

    assert_eq!(
        serde_json::to_value(report.summary).unwrap(),
        serde_json::to_value(expected_summary).unwrap()
    );
    assert_eq!(
        serde_json::to_value(report.drawdown).unwrap(),
        serde_json::to_value(expected_drawdown).unwrap()
    );
    assert_eq!(
        serde_json::to_value(report.attribution).unwrap(),
        serde_json::to_value(expected_attribution).unwrap()
    );
    assert_eq!(
        serde_json::to_value(report.monthly_returns).unwrap(),
        serde_json::to_value(expected_monthly_returns).unwrap()
    );
    assert_eq!(
        serde_json::to_value(report.holding_performances).unwrap(),
        serde_json::to_value(expected_holding_performances).unwrap()
    );
    assert_eq!(
        serde_json::to_value(report.risk_metrics).unwrap(),
        serde_json::to_value(expected_risk_metrics).unwrap()
    );
    assert_eq!(performance_load_count(), 1);
}

#[test]
fn test_performance_summary_neutralizes_in_kind_open_contribution() {
    let db = cash_flow_performance_db();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE daily_portfolio_values
                    SET total_value = 186, us_value = 186
                  WHERE date = '2024-01-03'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transactions
                 (id, holding_id, account_id, symbol, name, market, transaction_type,
                  shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES ('in-kind-open', NULL, 'acct-us', 'SPY', 'SPDR S&P 500 ETF', 'US', 'OPEN',
                         1, 20, 20, 1, 'USD', '2024-01-03T10:00:00Z', NULL, '2024-01-03')",
            [],
        )
        .unwrap();
    }

    let summary = get_performance_summary(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    // The additional USD 20 is an externally contributed position, not a
    // USD 20 investment gain. Performance remains 10% followed by 4.545%.
    assert!((summary.total_return - 15.0).abs() < 1e-9);
    assert!((summary.total_pnl - 15.0).abs() < 1e-9);
}

#[test]
fn test_performance_summary_neutralizes_cash_withdrawal_and_fee() {
    let db = cash_flow_performance_db();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE transactions
                    SET transaction_type = 'SELL', commission = 1
                  WHERE id = 'cash-in'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE daily_portfolio_values
                    SET total_value = 64, us_value = 64
                  WHERE date = '2024-01-03'",
            [],
        )
        .unwrap();
    }

    let summary = get_performance_summary(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    // Withdrawing USD 50 plus its USD 1 fee reduces account value by 51;
    // the remaining USD 5 change is performance, not a 41.8% loss.
    assert!((summary.return_series[1].daily_return - 4.545_454_545).abs() < 1e-9);
    assert!((summary.total_return - 15.0).abs() < 1e-9);
    assert!((summary.total_pnl - 15.0).abs() < 1e-9);
}

#[test]
fn test_monthly_return_geometrically_links_daily_twr_and_adjusts_pnl() {
    let db = cash_flow_performance_db();

    let months = get_monthly_returns(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    assert_eq!(months.len(), 1);
    assert!((months[0].return_rate - 15.0).abs() < 1e-9);
    assert!((months[0].pnl - 15.0).abs() < 1e-9);
    assert!((months[0].start_value - 100.0).abs() < 1e-9);
    assert!((months[0].end_value - 165.0).abs() < 1e-9);
}

#[test]
fn test_attribution_includes_net_dividend_fees_and_fully_sold_holding() {
    let db = sold_holding_performance_db();

    let attribution = get_return_attribution(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    // 0 ending value - 100 starting value + 118 net sale + 4 net dividend.
    assert!((attribution.total_pnl - 22.0).abs() < 1e-9);
    assert_eq!(attribution.by_holding.len(), 1);
    assert_eq!(attribution.by_holding[0].name, "AAPL Apple");
    assert!((attribution.by_holding[0].pnl - 22.0).abs() < 1e-9);
}

#[test]
fn test_ranking_keeps_fully_sold_holding_and_includes_income_and_fees() {
    let db = sold_holding_performance_db();

    let ranking = get_holding_performance_ranking(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        "return_rate",
        10,
        &PerformanceFilter::default(),
    )
    .unwrap();

    assert_eq!(ranking.len(), 1);
    assert_eq!(ranking[0].symbol, "AAPL");
    assert!((ranking[0].pnl - 22.0).abs() < 1e-9);
    assert!((ranking[0].return_rate - 22.0).abs() < 1e-9);
}

#[test]
fn test_attribution_includes_holding_bought_and_sold_inside_period() {
    let db = roundtrip_holding_performance_db();

    let attribution = get_return_attribution(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    let microsoft = attribution
        .by_holding
        .iter()
        .find(|item| item.name == "MSFT Microsoft")
        .expect("round-trip holding must remain in attribution");
    // 118 net sale proceeds - 101 purchase outlay.
    assert!((microsoft.pnl - 17.0).abs() < 1e-9);
    assert!((attribution.total_pnl - 39.0).abs() < 1e-9);
    assert_eq!(attribution.by_market.len(), 1);
    assert_eq!(attribution.by_market[0].name, "🇺🇸 美股");
    assert!((attribution.by_market[0].pnl - 39.0).abs() < 1e-9);
}

#[test]
fn test_unfiltered_attribution_aligns_baseline_and_normalizes_currency_to_usd() {
    let db = multicurrency_attribution_db();

    let attribution = get_return_attribution(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    // The selected period starts from the prior valuation (Jan 1), matching
    // the summary. CNY 72 gain / 7.2 = USD 10, not a mixed-currency 72.
    assert!((attribution.total_pnl - 10.0).abs() < 1e-9);
    assert!((attribution.by_market[0].pnl - 10.0).abs() < 1e-9);
    assert!((attribution.by_holding[0].pnl - 10.0).abs() < 1e-9);
}

#[test]
fn test_attribution_keeps_duplicate_symbols_separate_until_after_currency_normalization() {
    let db = duplicate_symbol_multicurrency_db();

    let attribution = get_return_attribution(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    assert!((attribution.total_pnl - 20.0).abs() < 1e-9);
    assert_eq!(attribution.by_market.len(), 2);
    assert!(attribution
        .by_market
        .iter()
        .all(|item| (item.pnl - 10.0).abs() < 1e-9));
    assert_eq!(attribution.by_category.len(), 2);
    assert!(attribution
        .by_category
        .iter()
        .all(|item| (item.pnl - 10.0).abs() < 1e-9));
}

#[test]
fn test_ranking_uses_same_previous_valuation_baseline_as_summary() {
    let db = multicurrency_attribution_db();

    let ranking = get_holding_performance_ranking(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        "return_rate",
        10,
        &PerformanceFilter::default(),
    )
    .unwrap();

    assert_eq!(ranking.len(), 1);
    // Unfiltered ranking normalizes CNY 72 / 7.2 to USD 10 before
    // comparing positions from different markets.
    assert!((ranking[0].pnl - 10.0).abs() < 1e-9);
    assert!((ranking[0].return_rate - 10.0).abs() < 1e-9);
}

#[test]
fn test_unfiltered_ranking_normalizes_duplicate_symbols_before_sorting() {
    let db = duplicate_symbol_multicurrency_db();

    let ranking = get_holding_performance_ranking(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        "pnl",
        10,
        &PerformanceFilter::default(),
    )
    .unwrap();

    assert_eq!(ranking.len(), 2);
    assert!(ranking.iter().all(|item| (item.pnl - 10.0).abs() < 1e-9));
    assert!(ranking
        .iter()
        .all(|item| (item.return_rate - 10.0).abs() < 1e-9));
}

#[test]
fn test_undefined_risk_ratios_are_not_reported_as_zero() {
    let db = flat_performance_db();

    let metrics = get_risk_metrics(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        &PerformanceFilter::default(),
    )
    .unwrap();

    assert_eq!(metrics.sharpe_ratio, None);
    assert_eq!(metrics.calmar_ratio, None);
}

#[test]
fn test_ranking_labels_roundtrip_holding_from_current_metadata() {
    let db = roundtrip_holding_performance_db();

    let ranking = get_holding_performance_ranking(
        &db,
        parse_date("2024-01-02").unwrap(),
        parse_date("2024-01-03").unwrap(),
        "return_rate",
        10,
        &PerformanceFilter::default(),
    )
    .unwrap();

    let microsoft = ranking
        .iter()
        .find(|item| item.symbol == "MSFT")
        .expect("round-trip holding must remain in ranking");
    assert_eq!(microsoft.name, "Microsoft");
    assert_eq!(microsoft.market, "US");
    assert_eq!(microsoft.category_name, "未分类");
    assert!((microsoft.pnl - 17.0).abs() < 1e-9);
    assert!((microsoft.return_rate - (17.0 / 101.0 * 100.0)).abs() < 1e-9);
}

#[test]
fn test_benchmark_to_return_series_no_base() {
    use crate::models::performance::BenchmarkDataPoint;
    let points = vec![
        BenchmarkDataPoint {
            date: "2024-01-01".into(),
            close_price: 100.0,
            change_percent: 0.0,
        },
        BenchmarkDataPoint {
            date: "2024-01-02".into(),
            close_price: 105.0,
            change_percent: 5.0,
        },
        BenchmarkDataPoint {
            date: "2024-01-03".into(),
            close_price: 103.0,
            change_percent: -1.9,
        },
    ];
    let series = benchmark_to_return_series(&points, None);
    assert_eq!(series.len(), 3);
    // Without base_price the first point is the baseline → 0%
    assert!((series[0].cumulative_return - 0.0).abs() < 1e-6);
    assert!((series[1].cumulative_return - 5.0).abs() < 1e-6);
    assert!((series[2].cumulative_return - 3.0).abs() < 1e-6);
}

#[test]
fn test_benchmark_to_return_series_with_base() {
    use crate::models::performance::BenchmarkDataPoint;
    // Previous day close was 95 → first visible day already shows a return
    let points = vec![
        BenchmarkDataPoint {
            date: "2024-01-02".into(),
            close_price: 100.0,
            change_percent: 5.26,
        },
        BenchmarkDataPoint {
            date: "2024-01-03".into(),
            close_price: 105.0,
            change_percent: 5.0,
        },
    ];
    let series = benchmark_to_return_series(&points, Some(95.0));
    assert_eq!(series.len(), 2);
    // cumulative: (100 - 95) / 95 * 100 ≈ 5.263%
    assert!((series[0].cumulative_return - 5.263_157_894).abs() < 0.001);
    // cumulative: (105 - 95) / 95 * 100 ≈ 10.526%
    assert!((series[1].cumulative_return - 10.526_315_789).abs() < 0.001);
    // daily: (100 - 95) / 95 * 100 ≈ 5.263%
    assert!((series[0].daily_return - 5.263_157_894).abs() < 0.001);
    // daily: (105 - 100) / 100 * 100 = 5%
    assert!((series[1].daily_return - 5.0).abs() < 1e-6);
}

// ─────────────────────────────────────────────────────────────────────────
// Additional performance calculation tests
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_annualise_return_half_year() {
    // 5% return in 182.5 days → annualised ≈ (1.05)^2 - 1 ≈ 10.25%
    let ar = annualise_return(0.05, 183);
    // (1.05)^(365/183) - 1 ≈ 0.10189
    assert!(ar > 0.10 && ar < 0.11);
}

#[test]
fn test_annualise_return_zero_days() {
    assert_eq!(annualise_return(0.10, 0), 0.0);
    assert_eq!(annualise_return(0.10, -5), 0.0);
}

#[test]
fn test_volatility_constant_returns() {
    // All same returns → variance = 0
    let returns = vec![1.0, 1.0, 1.0, 1.0, 1.0];
    let (dv, av) = calculate_volatility(&returns);
    assert!((dv - 0.0).abs() < 1e-9);
    assert!((av - 0.0).abs() < 1e-9);
}

#[test]
fn test_volatility_single_return() {
    let (dv, av) = calculate_volatility(&[5.0]);
    assert_eq!(dv, 0.0);
    assert_eq!(av, 0.0);
}

#[test]
fn test_volatility_empty() {
    let (dv, av) = calculate_volatility(&[]);
    assert_eq!(dv, 0.0);
    assert_eq!(av, 0.0);
}

#[test]
fn test_sharpe_uses_mean_daily_excess_return_and_sample_volatility() {
    let returns = [0.01, 0.02, -0.01];

    let sharpe = calculate_sharpe_from_daily_returns(&returns, 0.0).unwrap();

    // mean = 0.0066667, sample stdev = 0.0152753;
    // annualised Sharpe = mean / stdev * sqrt(252).
    assert!((sharpe - 6.928_203_230).abs() < 1e-9);
}

#[test]
fn test_sharpe_is_undefined_without_return_variability() {
    assert_eq!(
        calculate_sharpe_from_daily_returns(&[0.01, 0.01, 0.01], 0.0),
        None
    );
}

#[test]
fn test_max_drawdown_no_drawdown() {
    // Monotonically increasing portfolio
    let series: Vec<ReturnDataPoint> = vec![
        ReturnDataPoint {
            date: "2024-01-01".into(),
            cumulative_return: 0.0,
            daily_return: 0.0,
            portfolio_value: 100.0,
            daily_pnl: 0.0,
        },
        ReturnDataPoint {
            date: "2024-01-02".into(),
            cumulative_return: 5.0,
            daily_return: 5.0,
            portfolio_value: 105.0,
            daily_pnl: 5.0,
        },
        ReturnDataPoint {
            date: "2024-01-03".into(),
            cumulative_return: 10.0,
            daily_return: 5.0,
            portfolio_value: 110.0,
            daily_pnl: 5.0,
        },
    ];
    let dd = calculate_max_drawdown(&series, None);
    assert!((dd.max_drawdown - 0.0).abs() < 1e-9);
}

#[test]
fn test_max_drawdown_empty() {
    let dd = calculate_max_drawdown(&[], None);
    assert!((dd.max_drawdown - 0.0).abs() < 1e-9);
    assert_eq!(dd.drawdown_duration, 0);
}

#[test]
fn test_max_drawdown_with_recovery() {
    let series: Vec<ReturnDataPoint> = vec![
        ReturnDataPoint {
            date: "2024-01-01".into(),
            cumulative_return: 0.0,
            daily_return: 0.0,
            portfolio_value: 100.0,
            daily_pnl: 0.0,
        },
        ReturnDataPoint {
            date: "2024-01-02".into(),
            cumulative_return: 20.0,
            daily_return: 20.0,
            portfolio_value: 120.0,
            daily_pnl: 20.0,
        },
        ReturnDataPoint {
            date: "2024-01-03".into(),
            cumulative_return: 0.0,
            daily_return: -16.67,
            portfolio_value: 100.0,
            daily_pnl: -20.0,
        },
        ReturnDataPoint {
            date: "2024-01-04".into(),
            cumulative_return: 25.0,
            daily_return: 25.0,
            portfolio_value: 125.0,
            daily_pnl: 25.0,
        },
    ];
    let dd = calculate_max_drawdown(&series, None);
    // Peak=120, trough=100 → (100-120)/120 = -16.67%
    assert!((dd.max_drawdown - (-16.666_666_667)).abs() < 0.01);
    assert_eq!(dd.peak_date, "2024-01-02");
    assert_eq!(dd.trough_date, "2024-01-03");
    // Recovery at day 4 when value=125 >= peak of 120
    assert_eq!(dd.recovery_date, Some("2024-01-04".to_string()));
}

#[test]
fn test_max_drawdown_uses_twr_wealth_instead_of_raw_portfolio_value() {
    let series = vec![
        ReturnDataPoint {
            date: "2024-01-01".into(),
            cumulative_return: 0.0,
            daily_return: 0.0,
            portfolio_value: 100.0,
            daily_pnl: 0.0,
        },
        ReturnDataPoint {
            date: "2024-01-02".into(),
            cumulative_return: 0.0,
            daily_return: 0.0,
            portfolio_value: 200.0,
            daily_pnl: 0.0,
        },
        ReturnDataPoint {
            date: "2024-01-03".into(),
            cumulative_return: 0.0,
            daily_return: 0.0,
            portfolio_value: 100.0,
            daily_pnl: 0.0,
        },
    ];

    let drawdown = calculate_max_drawdown(&series, None);

    // The raw value halved because of a withdrawal, but investment wealth
    // stayed flat after neutralising the external cash flows.
    assert!((drawdown.max_drawdown - 0.0).abs() < 1e-9);
    assert!(drawdown
        .drawdown_series
        .iter()
        .all(|point| point.drawdown.abs() < 1e-9));
}

#[test]
fn test_max_drawdown_includes_loss_from_baseline_to_first_visible_day() {
    let series = vec![
        ReturnDataPoint {
            date: "2024-01-02".into(),
            cumulative_return: -10.0,
            daily_return: -10.0,
            portfolio_value: 90.0,
            daily_pnl: -10.0,
        },
        ReturnDataPoint {
            date: "2024-01-03".into(),
            cumulative_return: -5.0,
            daily_return: 5.555_555_556,
            portfolio_value: 95.0,
            daily_pnl: 5.0,
        },
    ];

    let drawdown = calculate_max_drawdown(&series, Some(parse_date("2024-01-01").unwrap()));

    assert!((drawdown.max_drawdown - (-10.0)).abs() < 1e-9);
    assert!((drawdown.drawdown_series[0].drawdown - (-10.0)).abs() < 1e-9);
    assert_eq!(drawdown.peak_date, "2024-01-01");
    assert_eq!(drawdown.trough_date, "2024-01-02");
    assert_eq!(drawdown.drawdown_duration, 1);
    assert_eq!(drawdown.recovery_date, None);
}
