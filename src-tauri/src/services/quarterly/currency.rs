use crate::models::quarterly::{QuarterlyHoldingSnapshot, QuarterlySnapshot};
use serde::Deserialize;

/// Rates frozen with one snapshot, expressed as units of currency per USD.
/// Missing pairs are allowed only when the requested conversion does not use them.
#[derive(Deserialize)]
pub(crate) struct SnapshotRates {
    usd_cny: Option<f64>,
    usd_hkd: Option<f64>,
}

impl SnapshotRates {
    pub(crate) fn from_json(json: &str) -> Result<Self, String> {
        let rates: Self = serde_json::from_str(json)
            .map_err(|error| format!("Invalid quarterly snapshot exchange rates: {error}"))?;
        for (pair, value) in [("USD/CNY", rates.usd_cny), ("USD/HKD", rates.usd_hkd)] {
            if let Some(value) = value {
                if !value.is_finite() || value <= 0.0 {
                    return Err(format!(
                        "Invalid quarterly snapshot exchange rate {pair}: {value}"
                    ));
                }
            }
        }
        Ok(rates)
    }

    fn per_usd(&self, currency: &str) -> Result<f64, String> {
        match currency {
            "USD" => Ok(1.0),
            "CNY" => self
                .usd_cny
                .ok_or_else(|| "Missing quarterly snapshot exchange rate USD/CNY".to_string()),
            "HKD" => self
                .usd_hkd
                .ok_or_else(|| "Missing quarterly snapshot exchange rate USD/HKD".to_string()),
            _ => Err(format!(
                "Unsupported quarterly snapshot currency: {currency}"
            )),
        }
    }

    pub(crate) fn convert(&self, amount: f64, from: &str, to: &str) -> Result<f64, String> {
        if !amount.is_finite() {
            return Err("Non-finite amount in quarterly snapshot".to_string());
        }
        let from = supported_currency(from)?;
        let to = supported_currency(to)?;
        let result = if amount == 0.0 || from == to {
            amount
        } else {
            amount / self.per_usd(&from)? * self.per_usd(&to)?
        };
        if !result.is_finite() {
            return Err(format!(
                "Non-finite quarterly snapshot conversion {from}/{to}"
            ));
        }
        Ok(result)
    }
}

fn supported_currency(currency: &str) -> Result<String, String> {
    let normalized = currency.trim().to_uppercase();
    match normalized.as_str() {
        "USD" | "CNY" | "HKD" => Ok(normalized),
        _ => Err(format!(
            "Unsupported quarterly snapshot currency: {currency}"
        )),
    }
}

/// Legacy rows have no currency. Cash symbols carry the actual currency even
/// when the account belongs to a different market.
pub(crate) fn currency_for_holding(
    symbol: &str,
    market: &str,
    explicit: &str,
) -> Result<String, String> {
    if !explicit.trim().is_empty() {
        return supported_currency(explicit);
    }
    let normalized_symbol = symbol.to_uppercase();
    if let Some(currency) = normalized_symbol.strip_prefix("$CASH-") {
        return supported_currency(currency);
    }
    match market {
        "US" => Ok("USD".to_string()),
        "CN" => Ok("CNY".to_string()),
        "HK" => Ok("HKD".to_string()),
        _ => Err(format!(
            "Cannot determine quarterly snapshot currency for market: {market}"
        )),
    }
}

pub(super) fn resolve_holding_currencies(
    holdings: &mut [QuarterlyHoldingSnapshot],
) -> Result<(), String> {
    for holding in holdings {
        holding.currency =
            currency_for_holding(&holding.symbol, &holding.market, &holding.currency)?;
    }
    Ok(())
}

/// A temporary view for common-axis report aggregates. Persisted holding rows
/// and the raw detail API retain their actual currency.
pub(super) fn holdings_in_usd(
    snapshot: &QuarterlySnapshot,
    holdings: &[QuarterlyHoldingSnapshot],
) -> Result<Vec<QuarterlyHoldingSnapshot>, String> {
    convert_holdings(snapshot, holdings, |_| Ok("USD".to_string()))
}

/// Holding-change rows are shown in each market's native currency.
pub(super) fn holdings_in_market_currency(
    snapshot: &QuarterlySnapshot,
    holdings: &[QuarterlyHoldingSnapshot],
) -> Result<Vec<QuarterlyHoldingSnapshot>, String> {
    convert_holdings(snapshot, holdings, |holding| {
        currency_for_holding("", &holding.market, "")
    })
}

fn convert_holdings(
    snapshot: &QuarterlySnapshot,
    holdings: &[QuarterlyHoldingSnapshot],
    target_currency: impl Fn(&QuarterlyHoldingSnapshot) -> Result<String, String>,
) -> Result<Vec<QuarterlyHoldingSnapshot>, String> {
    let convert = || {
        let rates = SnapshotRates::from_json(&snapshot.exchange_rates)?;
        holdings
            .iter()
            .map(|holding| {
                let currency =
                    currency_for_holding(&holding.symbol, &holding.market, &holding.currency)?;
                let target = target_currency(holding)?;
                let mut converted = holding.clone();
                converted.market_value = rates.convert(holding.market_value, &currency, &target)?;
                converted.cost_value = rates.convert(holding.cost_value, &currency, &target)?;
                converted.pnl = converted.market_value - converted.cost_value;
                converted.currency = target;
                Ok(converted)
            })
            .collect::<Result<Vec<_>, String>>()
    };
    convert().map_err(|error| format!("Quarter {}: {error}", snapshot.quarter))
}
