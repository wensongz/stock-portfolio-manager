use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Quarter helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the quarter string (e.g., "2025-Q1") for a given date.
pub fn date_to_quarter(date: NaiveDate) -> String {
    let q = (date.month() - 1) / 3 + 1;
    format!("{}-Q{}", date.year(), q)
}

/// Returns the last day of the quarter.
pub fn quarter_end_date(year: i32, q: u32) -> NaiveDate {
    match q {
        1 => NaiveDate::from_ymd_opt(year, 3, 31).unwrap(),
        2 => NaiveDate::from_ymd_opt(year, 6, 30).unwrap(),
        3 => NaiveDate::from_ymd_opt(year, 9, 30).unwrap(),
        4 => NaiveDate::from_ymd_opt(year, 12, 31).unwrap(),
        _ => unreachable!(),
    }
}

/// Parse a quarter string like "2025-Q1" into (year, quarter_number).
pub fn parse_quarter(s: &str) -> Result<(i32, u32), String> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid quarter format: '{}'", s));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| format!("Invalid year in quarter '{}'", s))?;
    let q_str = parts[1];
    if q_str.len() != 2 || !q_str.starts_with('Q') {
        return Err(format!("Invalid quarter part in '{}'", s));
    }
    let q: u32 = q_str[1..]
        .parse()
        .map_err(|_| format!("Invalid quarter number in '{}'", s))?;
    if !(1..=4).contains(&q) {
        return Err(format!("Quarter number must be 1-4, got {}", q));
    }
    Ok((year, q))
}

/// Returns the previous quarter string (e.g., "2025-Q1" -> "2024-Q4", "2025-Q3" -> "2025-Q2").
pub fn previous_quarter(s: &str) -> Result<String, String> {
    let (year, q) = parse_quarter(s)?;
    if q == 1 {
        Ok(format!("{}-Q4", year - 1))
    } else {
        Ok(format!("{}-Q{}", year, q - 1))
    }
}

/// Returns the first day of the quarter.
pub fn quarter_start_date(year: i32, q: u32) -> NaiveDate {
    match q {
        1 => NaiveDate::from_ymd_opt(year, 1, 1).unwrap(),
        2 => NaiveDate::from_ymd_opt(year, 4, 1).unwrap(),
        3 => NaiveDate::from_ymd_opt(year, 7, 1).unwrap(),
        4 => NaiveDate::from_ymd_opt(year, 10, 1).unwrap(),
        _ => unreachable!(),
    }
}
