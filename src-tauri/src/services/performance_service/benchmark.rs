use crate::db::Database;
use crate::models::performance::{BenchmarkDataPoint, ReturnDataPoint};
use crate::services::http_client;
use chrono::NaiveDate;
use std::future::Future;

/// Replace just the successfully refreshed interval, as one atomic write.
pub fn cache_benchmark_prices(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
    points: &[BenchmarkDataPoint],
) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    let transaction = conn.transaction().map_err(|e| e.to_string())?;
    transaction
        .execute(
            "DELETE FROM benchmark_daily_prices WHERE symbol = ?1 AND date BETWEEN ?2 AND ?3",
            rusqlite::params![symbol, start_date.to_string(), end_date.to_string()],
        )
        .map_err(|e| e.to_string())?;
    for p in points {
        transaction
            .execute(
                "INSERT INTO benchmark_daily_prices (symbol, date, close_price, change_percent)
             VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![symbol, p.date, p.close_price, p.change_percent],
            )
            .map_err(|e| e.to_string())?;
    }
    transaction.commit().map_err(|e| e.to_string())
}

/// Read cached benchmark prices from SQLite.
pub fn read_cached_benchmark(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_date.format("%Y-%m-%d").to_string();
    let end_str = end_date.format("%Y-%m-%d").to_string();

    let mut stmt = conn
        .prepare(
            "SELECT date, close_price, change_percent
             FROM benchmark_daily_prices
             WHERE symbol = ?1 AND date BETWEEN ?2 AND ?3
             ORDER BY date ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![symbol, start_str, end_str], |row| {
            Ok(BenchmarkDataPoint {
                date: row.get(0)?,
                close_price: row.get(1)?,
                change_percent: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows)
}

/// Refresh closes from Yahoo Finance. Row counts cannot distinguish a partial
/// cache from a completed window over an exchange's holiday schedule.
pub async fn fetch_benchmark_history(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    if start_date > end_date {
        return Ok(vec![]);
    }
    refresh_benchmark_history_with(
        db,
        symbol,
        start_date,
        end_date,
        download_benchmark_history(symbol, start_date, end_date),
    )
    .await
}

async fn refresh_benchmark_history_with(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
    download: impl Future<Output = Result<Vec<BenchmarkDataPoint>, String>>,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    let points = download.await?;
    cache_benchmark_prices(db, symbol, start_date, end_date, &points)?;
    Ok(points)
}

async fn download_benchmark_history(
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    let start_ts = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_ts = end_date
        .succ_opt()
        .ok_or_else(|| "基准行情结束日期超出范围".to_string())?
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();

    let mut url = reqwest::Url::parse("https://query1.finance.yahoo.com/v8/finance/chart/")
        .map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "基准行情 URL 无效".to_string())?
        .pop_if_empty()
        .push(symbol);
    url.query_pairs_mut()
        .append_pair("period1", &start_ts.to_string())
        .append_pair("period2", &end_ts.to_string())
        .append_pair("interval", "1d");

    let resp = http_client::general_client()
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    parse_benchmark_history(&json, start_date, end_date)
}

fn parse_benchmark_history(
    json: &serde_json::Value,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    let result = &json["chart"]["result"][0];
    let timestamps = result["timestamp"]
        .as_array()
        .ok_or_else(|| "基准行情缺少交易日期".to_string())?;
    let closes = result["indicators"]["quote"][0]["close"]
        .as_array()
        .ok_or_else(|| "基准行情缺少收盘价".to_string())?;
    if timestamps.len() != closes.len() {
        return Err("基准行情交易日期与收盘价数量不符".to_string());
    }

    let mut daily_prices = std::collections::BTreeMap::new();
    for (timestamp, close) in timestamps.iter().zip(closes) {
        let date = timestamp
            .as_i64()
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .ok_or_else(|| "基准行情包含无效交易日期".to_string())?
            .date_naive();
        if date < start_date || date > end_date {
            continue;
        }
        let price = close
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or_else(|| format!("基准行情缺少 {date} 的有效收盘价"))?;
        daily_prices.insert(date, price);
    }

    let mut previous = None;
    Ok(daily_prices
        .into_iter()
        .map(|(date, close_price)| {
            let change_percent = previous.map_or(0.0, |price| (close_price / price - 1.0) * 100.0);
            previous = Some(close_price);
            BenchmarkDataPoint {
                date: date.to_string(),
                close_price,
                change_percent,
            }
        })
        .collect())
}

/// Build a return series for the benchmark (cumulative %).
/// When `base_price` is provided, cumulative returns are calculated relative
/// to this price (the closing price on the day before the visible range)
/// instead of the first element in `points`.
pub fn benchmark_to_return_series(
    points: &[BenchmarkDataPoint],
    base_price: Option<f64>,
) -> Vec<ReturnDataPoint> {
    if points.is_empty() {
        return vec![];
    }
    let start_price = base_price.unwrap_or(points[0].close_price);
    let mut prev_price = start_price;
    points
        .iter()
        .map(|p| {
            let daily_return = if prev_price > 0.0 {
                (p.close_price - prev_price) / prev_price * 100.0
            } else {
                0.0
            };
            let cumulative_return = if start_price > 0.0 {
                (p.close_price - start_price) / start_price * 100.0
            } else {
                0.0
            };
            prev_price = p.close_price;
            ReturnDataPoint {
                date: p.date.clone(),
                cumulative_return,
                daily_return,
                portfolio_value: p.close_price,
                daily_pnl: 0.0,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(day: u32, close_price: f64) -> BenchmarkDataPoint {
        BenchmarkDataPoint {
            date: date(day).to_string(),
            close_price,
            change_percent: 0.0,
        }
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    #[tokio::test]
    async fn refreshing_downloads_missing_closes_despite_a_high_cache_row_count() {
        let db = Database::new(":memory:").unwrap();
        let symbol = "^GSPC";
        cache_benchmark_prices(
            &db,
            symbol,
            date(10),
            date(16),
            &[point(10, 99.0), point(11, 100.0), point(13, 102.0)],
        )
        .unwrap();
        let complete = vec![
            point(10, 99.0),
            point(11, 100.0),
            point(12, 101.0),
            point(13, 102.0),
            point(16, 106.0),
        ];
        let results =
            refresh_benchmark_history_with(&db, symbol, date(10), date(16), async { Ok(complete) })
                .await
                .unwrap();
        assert_eq!(results.len(), 5);
        let cached = read_cached_benchmark(&db, symbol, date(10), date(16)).unwrap();
        assert_eq!(cached[2].date, "2026-11-12");
        assert_eq!(cached.last().unwrap().close_price, 106.0);
    }

    #[tokio::test]
    async fn failed_refresh_keeps_prior_cache_and_success_replaces_only_its_range() {
        let db = Database::new(":memory:").unwrap();
        let symbol = "^GSPC";
        cache_benchmark_prices(
            &db,
            symbol,
            date(10),
            date(16),
            &[point(10, 99.0), point(11, 100.0), point(13, 102.0)],
        )
        .unwrap();
        let failed = refresh_benchmark_history_with(&db, symbol, date(11), date(13), async {
            Err("network".to_string())
        })
        .await;
        assert!(failed.is_err());
        assert_eq!(
            read_cached_benchmark(&db, symbol, date(10), date(16))
                .unwrap()
                .len(),
            3
        );

        refresh_benchmark_history_with(&db, symbol, date(11), date(13), async {
            Ok(vec![point(13, 105.0)])
        })
        .await
        .unwrap();
        let cached = read_cached_benchmark(&db, symbol, date(10), date(16)).unwrap();
        assert_eq!(
            cached
                .iter()
                .map(|point| point.close_price)
                .collect::<Vec<_>>(),
            vec![99.0, 105.0]
        );
    }
}
