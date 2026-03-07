use crate::db::Database;
use crate::models::holding::{CreateHoldingRequest, Holding, UpdateHoldingRequest};
use uuid::Uuid;

pub fn create_holding(db: &Database, req: CreateHoldingRequest) -> Result<Holding, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    conn.execute(
        "INSERT INTO holdings (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            id,
            req.account_id,
            req.symbol,
            req.name,
            req.market,
            req.category_id,
            req.shares,
            req.avg_cost,
            req.currency,
            now,
            now,
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(Holding {
        id,
        account_id: req.account_id,
        symbol: req.symbol,
        name: req.name,
        market: req.market,
        category_id: req.category_id,
        shares: req.shares,
        avg_cost: req.avg_cost,
        currency: req.currency,
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn list_holdings(db: &Database, account_id: Option<&str>) -> Result<Vec<Holding>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match account_id {
        Some(aid) => (
            "SELECT id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at
             FROM holdings WHERE account_id = ?1 ORDER BY symbol",
            vec![Box::new(aid.to_string())],
        ),
        None => (
            "SELECT id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at
             FROM holdings ORDER BY market, symbol",
            vec![],
        ),
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let holdings = stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok(Holding {
                id: row.get(0)?,
                account_id: row.get(1)?,
                symbol: row.get(2)?,
                name: row.get(3)?,
                market: row.get(4)?,
                category_id: row.get(5)?,
                shares: row.get(6)?,
                avg_cost: row.get(7)?,
                currency: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(holdings)
}

pub fn get_holding(db: &Database, id: &str) -> Result<Holding, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at
         FROM holdings WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(Holding {
                id: row.get(0)?,
                account_id: row.get(1)?,
                symbol: row.get(2)?,
                name: row.get(3)?,
                market: row.get(4)?,
                category_id: row.get(5)?,
                shares: row.get(6)?,
                avg_cost: row.get(7)?,
                currency: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

pub fn update_holding(db: &Database, id: &str, req: UpdateHoldingRequest) -> Result<Holding, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let existing = conn
        .query_row(
            "SELECT id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at
             FROM holdings WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Holding {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    symbol: row.get(2)?,
                    name: row.get(3)?,
                    market: row.get(4)?,
                    category_id: row.get(5)?,
                    shares: row.get(6)?,
                    avg_cost: row.get(7)?,
                    currency: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    let name = req.name.unwrap_or(existing.name);
    let category_id = if req.category_id.is_some() { req.category_id } else { existing.category_id };
    let shares = req.shares.unwrap_or(existing.shares);
    let avg_cost = req.avg_cost.unwrap_or(existing.avg_cost);

    conn.execute(
        "UPDATE holdings SET name = ?1, category_id = ?2, shares = ?3, avg_cost = ?4, updated_at = ?5 WHERE id = ?6",
        rusqlite::params![name, category_id, shares, avg_cost, now, id],
    )
    .map_err(|e| e.to_string())?;

    Ok(Holding {
        id: id.to_string(),
        account_id: existing.account_id,
        symbol: existing.symbol,
        name,
        market: existing.market,
        category_id,
        shares,
        avg_cost,
        currency: existing.currency,
        created_at: existing.created_at,
        updated_at: now,
    })
}

pub fn delete_holding(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let changes = conn
        .execute("DELETE FROM holdings WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;

    if changes == 0 {
        return Err("Holding not found".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::account::CreateAccountRequest;
    use crate::services::account_service;

    fn setup_db_with_account() -> (Database, String) {
        let db = Database::new_in_memory().unwrap();
        let account = account_service::create_account(
            &db,
            CreateAccountRequest {
                name: "Test Account".to_string(),
                market: "US".to_string(),
                description: None,
            },
        )
        .unwrap();
        (db, account.id)
    }

    #[test]
    fn test_create_and_list_holdings() {
        let (db, account_id) = setup_db_with_account();

        let holding = create_holding(
            &db,
            CreateHoldingRequest {
                account_id: account_id.clone(),
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                category_id: Some("cat-growth".to_string()),
                shares: 100.0,
                avg_cost: 150.0,
                currency: "USD".to_string(),
            },
        )
        .unwrap();

        assert_eq!(holding.symbol, "AAPL");
        assert_eq!(holding.shares, 100.0);

        let holdings = list_holdings(&db, Some(&account_id)).unwrap();
        assert_eq!(holdings.len(), 1);
    }

    #[test]
    fn test_update_holding() {
        let (db, account_id) = setup_db_with_account();

        let holding = create_holding(
            &db,
            CreateHoldingRequest {
                account_id,
                symbol: "MSFT".to_string(),
                name: "Microsoft".to_string(),
                market: "US".to_string(),
                category_id: None,
                shares: 50.0,
                avg_cost: 300.0,
                currency: "USD".to_string(),
            },
        )
        .unwrap();

        let updated = update_holding(
            &db,
            &holding.id,
            UpdateHoldingRequest {
                name: None,
                category_id: Some("cat-dividend".to_string()),
                shares: Some(75.0),
                avg_cost: Some(280.0),
            },
        )
        .unwrap();

        assert_eq!(updated.shares, 75.0);
        assert_eq!(updated.avg_cost, 280.0);
        assert_eq!(updated.category_id, Some("cat-dividend".to_string()));
    }

    #[test]
    fn test_delete_holding() {
        let (db, account_id) = setup_db_with_account();

        let holding = create_holding(
            &db,
            CreateHoldingRequest {
                account_id,
                symbol: "TSLA".to_string(),
                name: "Tesla".to_string(),
                market: "US".to_string(),
                category_id: None,
                shares: 10.0,
                avg_cost: 200.0,
                currency: "USD".to_string(),
            },
        )
        .unwrap();

        delete_holding(&db, &holding.id).unwrap();
        let result = get_holding(&db, &holding.id);
        assert!(result.is_err());
    }
}
