use crate::db::Database;
use crate::models::{ExchangeRates, StockQuote};
use crate::services::cash_reconciliation_service::get_cash_balance_reconciliation;
use crate::services::portfolio_mutation::{
    create_holding_in, create_transaction_in, update_transaction_in, CreateHoldingInput,
    CreateTransactionInput,
};
use crate::services::portfolio_read_service::{PortfolioReadModel, QuoteReadMode};
use crate::services::quote_service::QuoteCache;
use crate::services::statistics_service::{by_account, by_market};

fn database(market: &str) -> Database {
    let db = Database::new(":memory:").unwrap();
    db.conn.lock().unwrap().execute("INSERT INTO accounts(id,name,market,created_at,updated_at) VALUES('mixed','Mixed Broker',?1,'2026-01-01','2026-01-01')", [market]).unwrap();
    db
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
}

fn trade(symbol: &str, market: &str, currency: &str, amount: f64) -> CreateTransactionInput {
    CreateTransactionInput {
        account_id: "mixed".into(),
        symbol: symbol.into(),
        name: symbol.into(),
        market: market.into(),
        currency: currency.into(),
        transaction_type: "BUY".into(),
        shares: 1.0,
        price: amount,
        total_amount: amount,
        commission: 1.0,
        traded_at: "2026-11-03T10:00:00Z".into(),
        notes: None,
    }
}

#[test]
fn cash_openings_follow_currency_when_account_market_differs() {
    for (currency, expected_market) in [("HKD", "HK"), ("USD", "US"), ("CNY", "CN")] {
        let db = database("US");
        let conn = db.conn.lock().unwrap();
        let holding = create_holding_in(
            &conn,
            &CreateHoldingInput {
                account_id: "mixed".into(),
                symbol: format!("$CASH-{currency}"),
                name: "Cash".into(),
                market: "CN".into(),
                currency: currency.into(),
                shares: 1000.0,
                avg_cost: 1.0,
                category_id: None,
            },
        )
        .unwrap();
        assert_eq!(holding.market, expected_market);
        let opening_market: String = conn
            .query_row(
                "SELECT market FROM transactions WHERE holding_id=?1",
                [&holding.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(opening_market, expected_market);
    }
}

#[test]
fn cash_deposit_edits_keep_separate_currency_balances_and_repair_existing_market() {
    let db = database("US");
    let mut conn = db.conn.lock().unwrap();
    conn.execute_batch("INSERT INTO holdings(id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES('legacy','mixed','$CASH-HKD','Cash','US',1000,1,'HKD','2026-01-01','2026-01-01');").unwrap();
    let tx = conn.transaction().unwrap();
    let mut deposit = trade("$CASH-HKD", "US", "HKD", 100.0);
    deposit.shares = 0.0;
    deposit.price = 0.0;
    deposit.commission = 0.0;
    let first = create_transaction_in(&tx, &deposit).unwrap();
    assert_eq!(first.market, "HK");
    let (market, shares): (String, f64) = tx
        .query_row(
            "SELECT market,shares FROM holdings WHERE id='legacy'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(market, "HK");
    near(shares, 1100.0);
    deposit.symbol = "$CASH-USD".into();
    deposit.currency = "USD".into();
    deposit.market = "HK".into();
    deposit.total_amount = 50.0;
    let updated = update_transaction_in(&tx, &first.id, &deposit).unwrap();
    assert_eq!(updated.market, "US");
    let balances: Vec<(String, String, f64)> = tx
        .prepare("SELECT currency,market,shares FROM holdings ORDER BY currency")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        balances,
        vec![
            ("HKD".into(), "HK".into(), 1000.0),
            ("USD".into(), "US".into(), 50.0)
        ]
    );
    tx.commit().unwrap();
}

#[tokio::test]
async fn market_statistics_classify_legacy_cash_by_currency_without_rewriting_history() {
    let db = database("US");
    db.conn.lock().unwrap().execute_batch("INSERT INTO holdings(id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES('legacy','mixed','$CASH-HKD','Cash','US',7800,1,'HKD','2026-01-01','2026-01-01');").unwrap();
    let model = PortfolioReadModel::load(&db, &QuoteCache::new(), None, QuoteReadMode::CacheOnly)
        .await
        .unwrap();
    near(by_market(&model, "US").total_market_value, 0.0);
    near(by_market(&model, "HK").total_market_value, 7800.0);
    assert_eq!(by_market(&model, "HK").holdings[0].currency, "HKD");
}

#[tokio::test]
async fn cross_currency_account_statistics_reject_invalid_required_exchange_rates() {
    for (market, currency, invalid_leg) in [
        ("US", "HKD", "hkd"),
        ("HK", "USD", "hkd"),
        ("HK", "CNY", "cny"),
        ("CN", "HKD", "cny"),
        ("US", "CNY", "cny"),
    ] {
        let db = database(market);
        db.conn.lock().unwrap().execute("INSERT INTO holdings(id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES('cash','mixed',?1,'Cash',?2,1000,1,?3,'2026-01-01','2026-01-01')",rusqlite::params![format!("$CASH-{currency}"),market,currency]).unwrap();
        let model =
            PortfolioReadModel::load(&db, &QuoteCache::new(), None, QuoteReadMode::CacheOnly)
                .await
                .unwrap();
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut rates = ExchangeRates {
                usd_cny: 7.0,
                usd_hkd: 7.8,
                cny_hkd: 7.8 / 7.0,
                updated_at: "2026-11-03".into(),
            };
            if invalid_leg == "hkd" {
                rates.usd_hkd = invalid;
            } else {
                rates.usd_cny = invalid;
            }
            assert!(
                by_account(&model, Some(&rates), "mixed", market).is_err(),
                "{currency} to {market} accepted invalid rate {invalid}"
            );
        }
    }
}

#[tokio::test]
async fn foreign_stock_purchase_keeps_negative_currency_cash_in_market_and_account_totals() {
    let db = database("US");
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        create_holding_in(
            &tx,
            &CreateHoldingInput {
                account_id: "mixed".into(),
                symbol: "$CASH-USD".into(),
                name: "USD Cash".into(),
                market: "US".into(),
                currency: "USD".into(),
                shares: 1000.0,
                avg_cost: 1.0,
                category_id: None,
            },
        )
        .unwrap();
        create_transaction_in(&tx, &trade("0700.HK", "HK", "HKD", 100.0)).unwrap();
        tx.commit().unwrap();
    }
    let quotes = QuoteCache::new();
    quotes.set(StockQuote {
        symbol: "0700.HK".into(),
        market: "HK".into(),
        current_price: 100.0,
        ..Default::default()
    });
    let model = PortfolioReadModel::load(&db, &quotes, None, QuoteReadMode::CacheOnly)
        .await
        .unwrap();
    let hkd_cash = model
        .holdings()
        .iter()
        .find(|h| h.symbol == "$CASH-HKD")
        .expect("a debit cash balance is part of net assets");
    near(hkd_cash.market_value, -101.0);
    near(by_market(&model, "US").total_market_value, 1000.0);
    near(by_market(&model, "HK").total_market_value, -1.0);
    let rates = ExchangeRates {
        usd_cny: 7.0,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.0,
        updated_at: "2026-11-03".into(),
    };
    let account = by_account(&model, Some(&rates), "mixed", "US").unwrap();
    near(account.total_market_value, 999.8717948717949);
    near(account.total_cost, 1000.0);
    near(account.total_pnl, -1.0 / 7.8);
}

#[tokio::test]
async fn same_account_us_hk_stocks_cash_totals_and_ledgers_stay_consistent() {
    for account_market in ["US", "HK"] {
        let db = database(account_market);
        let (usd_id, hkd_id) = {
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            let mut ids = Vec::new();
            for (symbol, market, currency, shares, cost) in [
                ("AAPL", "US", "USD", 10.0, 100.0),
                ("0700.HK", "HK", "HKD", 100.0, 200.0),
                ("$CASH-USD", "US", "USD", 1000.0, 1.0),
                ("$CASH-HKD", "HK", "HKD", 7800.0, 1.0),
            ] {
                let holding = create_holding_in(
                    &tx,
                    &CreateHoldingInput {
                        account_id: "mixed".into(),
                        symbol: symbol.into(),
                        name: symbol.into(),
                        market: market.into(),
                        currency: currency.into(),
                        shares,
                        avg_cost: cost,
                        category_id: None,
                    },
                )
                .unwrap();
                if symbol.starts_with("$CASH-") {
                    ids.push(holding.id);
                }
            }
            tx.execute(
                "UPDATE transactions SET traded_at='2026-01-01T00:00:00Z'",
                [],
            )
            .unwrap();
            tx.commit().unwrap();
            (ids[0].clone(), ids[1].clone())
        };
        let cache = QuoteCache::new();
        for (symbol, market, price) in [("AAPL", "US", 200.0), ("0700.HK", "HK", 300.0)] {
            cache.set(StockQuote {
                symbol: symbol.into(),
                market: market.into(),
                current_price: price,
                ..StockQuote::default()
            });
        }
        let model = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::CacheOnly)
            .await
            .unwrap();
        let us = by_market(&model, "US");
        let hk = by_market(&model, "HK");
        near(us.total_market_value, 3000.0);
        near(us.total_cost, 2000.0);
        near(us.total_pnl, 1000.0);
        near(hk.total_market_value, 37800.0);
        near(hk.total_cost, 27800.0);
        near(hk.total_pnl, 10000.0);
        assert_eq!(us.holdings.len(), 2);
        assert_eq!(hk.holdings.len(), 2);
        assert!(us
            .holdings
            .iter()
            .all(|h| h.account_id == "mixed" && h.currency == "USD"));
        assert!(hk
            .holdings
            .iter()
            .all(|h| h.account_id == "mixed" && h.currency == "HKD"));
        near(us.account_distribution[0].value, 3000.0);
        near(hk.account_distribution[0].value, 37800.0);
        let rates = ExchangeRates {
            usd_cny: 7.0,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.0,
            updated_at: "2026-11-03".into(),
        };
        let account = by_account(&model, Some(&rates), "mixed", account_market).unwrap();
        let factor = if account_market == "HK" {
            1.0
        } else {
            1.0 / 7.8
        };
        near(account.total_market_value, 61200.0 * factor);
        near(account.total_cost, 43400.0 * factor);
        near(account.total_pnl, 17800.0 * factor);
        near(
            account.category_distribution.iter().map(|s| s.value).sum(),
            account.total_market_value,
        );
        near(
            account.stock_distribution.iter().map(|s| s.value).sum(),
            account.total_market_value,
        );
        {
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            create_transaction_in(&tx, &trade("AAPL", "US", "USD", 100.0)).unwrap();
            create_transaction_in(&tx, &trade("0700.HK", "HK", "HKD", 200.0)).unwrap();
            tx.commit().unwrap();
        }
        let usd = get_cash_balance_reconciliation(&db, &usd_id).unwrap();
        let hkd = get_cash_balance_reconciliation(&db, &hkd_id).unwrap();
        near(usd.current_balance, 899.0);
        near(usd.recommended_balance.unwrap(), 899.0);
        near(hkd.current_balance, 7599.0);
        near(hkd.recommended_balance.unwrap(), 7599.0);
        assert!(usd.rows.iter().all(|r| r.transaction.currency == "USD"));
        assert!(hkd.rows.iter().all(|r| r.transaction.currency == "HKD"));
        let updated = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::CacheOnly)
            .await
            .unwrap();
        let us = by_market(&updated, "US");
        let hk = by_market(&updated, "HK");
        near(us.total_market_value, 3099.0);
        near(us.total_cost, 2000.0);
        near(hk.total_market_value, 37899.0);
        near(hk.total_cost, 27800.0);
        let account = by_account(&updated, Some(&rates), "mixed", account_market).unwrap();
        near(account.total_market_value, 62071.2 * factor);
        near(account.total_cost, 43400.0 * factor);
        near(account.total_pnl, 18671.2 * factor);
    }
}
