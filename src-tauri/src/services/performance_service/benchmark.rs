use crate::db::Database;
use crate::models::performance::{BenchmarkDataPoint, ReturnDataPoint};
use crate::services::http_client;
use chrono::NaiveDate;

const CACHE_COVERAGE_THRESHOLD: f64 = 0.5; // require 50% of expected days in cache to skip re-fetch

pub fn cache_benchmark_prices(
    db: &Database,
    symbol: &str,
    points: &[BenchmarkDataPoint],
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    for p in points {
        conn.execute(
            "INSERT OR REPLACE INTO benchmark_daily_prices (symbol, date, close_price, change_percent)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![symbol, p.date, p.close_price, p.change_percent],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
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

/// Fetch benchmark history from Yahoo Finance and cache it.
pub async fn fetch_benchmark_history(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    // Check cache first
    let cached = read_cached_benchmark(db, symbol, start_date, end_date)?;

    // If we have data covering the range, use it
    let days_needed = (end_date - start_date).num_days();
    if (cached.len() as f64) >= days_needed as f64 * CACHE_COVERAGE_THRESHOLD {
        return Ok(cached);
    }

    // Fetch from Yahoo Finance
    let start_ts = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_ts = end_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval=1d",
        symbol, start_ts, end_ts
    );

    let resp = http_client::general_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let timestamps = json["chart"]["result"][0]["timestamp"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let closes = json["chart"]["result"][0]["indicators"]["quote"][0]["close"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut points: Vec<BenchmarkDataPoint> = Vec::new();
    let mut prev_close: Option<f64> = None;

    for (ts, cl) in timestamps.iter().zip(closes.iter()) {
        if let (Some(ts_i), Some(cl_f)) = (ts.as_i64(), cl.as_f64()) {
            let date = chrono::DateTime::from_timestamp(ts_i, 0)
                .unwrap_or_default()
                .date_naive();
            let change_pct = prev_close
                .map(|pc| {
                    if pc != 0.0 {
                        (cl_f - pc) / pc * 100.0
                    } else {
                        0.0
                    }
                })
                .unwrap_or(0.0);
            points.push(BenchmarkDataPoint {
                date: date.format("%Y-%m-%d").to_string(),
                close_price: cl_f,
                change_percent: change_pct,
            });
            prev_close = Some(cl_f);
        }
    }

    // Cache the fetched data
    cache_benchmark_prices(db, symbol, &points)?;

    Ok(points)
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
