use crate::db::Database;
use crate::models::alert::{PriceAlert, TriggeredAlert};
use chrono::Utc;
use uuid::Uuid;

pub fn create_alert(
    db: &Database,
    holding_id: Option<String>,
    symbol: String,
    name: String,
    market: String,
    alert_type: String,
    threshold: f64,
) -> Result<PriceAlert, String> {
    let conn = db.conn.lock().unwrap();
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO price_alerts (id, holding_id, symbol, name, market, alert_type, threshold, is_active, is_triggered, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, 0, ?8)",
        rusqlite::params![id, holding_id, symbol, name, market, alert_type, threshold, now],
    )
    .map_err(|e| e.to_string())?;

    Ok(PriceAlert {
        id,
        holding_id,
        symbol,
        name,
        market,
        alert_type,
        threshold,
        is_active: true,
        is_triggered: false,
        triggered_at: None,
        created_at: now,
    })
}

pub fn get_alerts(db: &Database) -> Result<Vec<PriceAlert>, String> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, holding_id, symbol, name, market, alert_type, threshold,
                    is_active, is_triggered, triggered_at, created_at
             FROM price_alerts ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let alerts = stmt
        .query_map([], |row| {
            Ok(PriceAlert {
                id: row.get(0)?,
                holding_id: row.get(1)?,
                symbol: row.get(2)?,
                name: row.get(3)?,
                market: row.get(4)?,
                alert_type: row.get(5)?,
                threshold: row.get(6)?,
                is_active: row.get::<_, i32>(7)? != 0,
                is_triggered: row.get::<_, i32>(8)? != 0,
                triggered_at: row.get(9)?,
                created_at: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(alerts)
}

pub fn update_alert(db: &Database, id: &str, is_active: bool) -> Result<PriceAlert, String> {
    let conn = db.conn.lock().unwrap();

    conn.execute(
        "UPDATE price_alerts SET is_active = ?1 WHERE id = ?2",
        rusqlite::params![is_active as i32, id],
    )
    .map_err(|e| e.to_string())?;

    let alert = conn
        .query_row(
            "SELECT id, holding_id, symbol, name, market, alert_type, threshold,
                    is_active, is_triggered, triggered_at, created_at
             FROM price_alerts WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(PriceAlert {
                    id: row.get(0)?,
                    holding_id: row.get(1)?,
                    symbol: row.get(2)?,
                    name: row.get(3)?,
                    market: row.get(4)?,
                    alert_type: row.get(5)?,
                    threshold: row.get(6)?,
                    is_active: row.get::<_, i32>(7)? != 0,
                    is_triggered: row.get::<_, i32>(8)? != 0,
                    triggered_at: row.get(9)?,
                    created_at: row.get(10)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(alert)
}

pub fn delete_alert(db: &Database, id: &str) -> Result<bool, String> {
    let conn = db.conn.lock().unwrap();
    let rows = conn
        .execute("DELETE FROM price_alerts WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    Ok(rows > 0)
}

/// Check all active alerts against provided quote data.
/// `quotes` is a map of symbol -> (current_price, change_percent, pnl_percent)
pub fn check_alerts(
    db: &Database,
    quotes: &std::collections::HashMap<String, (f64, f64, f64)>,
) -> Result<Vec<TriggeredAlert>, String> {
    let alerts = get_alerts(db)?;
    let now = Utc::now().to_rfc3339();
    let mut triggered = Vec::new();

    let conn = db.conn.lock().unwrap();

    for alert in alerts {
        if !alert.is_active || alert.is_triggered {
            continue;
        }
        let Some(&(price, change_pct, pnl_pct)) = quotes.get(&alert.symbol) else {
            continue;
        };

        let (current_value, condition_met, message) = match alert.alert_type.as_str() {
            "PRICE_ABOVE" => (
                price,
                price > alert.threshold,
                format!("{} 价格 {:.2} 已超过 {:.2}", alert.name, price, alert.threshold),
            ),
            "PRICE_BELOW" => (
                price,
                price < alert.threshold,
                format!("{} 价格 {:.2} 已低于 {:.2}", alert.name, price, alert.threshold),
            ),
            "CHANGE_ABOVE" => (
                change_pct,
                change_pct > alert.threshold,
                format!("{} 涨幅 {:.2}% 已超过 {:.2}%", alert.name, change_pct, alert.threshold),
            ),
            "CHANGE_BELOW" => (
                change_pct,
                change_pct < alert.threshold,
                format!("{} 跌幅 {:.2}% 已低于 {:.2}%", alert.name, change_pct, alert.threshold),
            ),
            "PNL_ABOVE" => (
                pnl_pct,
                pnl_pct > alert.threshold,
                format!("{} 盈亏 {:.2}% 已超过 {:.2}%", alert.name, pnl_pct, alert.threshold),
            ),
            "PNL_BELOW" => (
                pnl_pct,
                pnl_pct < alert.threshold,
                format!("{} 盈亏 {:.2}% 已低于 {:.2}%", alert.name, pnl_pct, alert.threshold),
            ),
            _ => continue,
        };

        if condition_met {
            let _ = conn.execute(
                "UPDATE price_alerts SET is_triggered = 1, triggered_at = ?1 WHERE id = ?2",
                rusqlite::params![now, alert.id],
            );

            triggered.push(TriggeredAlert {
                alert: PriceAlert {
                    is_triggered: true,
                    triggered_at: Some(now.clone()),
                    ..alert
                },
                current_value,
                message,
            });
        }
    }

    Ok(triggered)
}
