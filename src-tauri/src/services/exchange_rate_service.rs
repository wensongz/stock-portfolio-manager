use crate::models::ExchangeRates;
use chrono::Utc;
use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CACHE_TTL_SECS: u64 = 10 * 60; // 10-minute cache

#[derive(Debug, Deserialize)]
struct ErApiResponse {
    result: Option<String>,
    rates: Option<std::collections::HashMap<String, f64>>,
}

/// In-memory cache for exchange rates.
pub struct ExchangeRateCache {
    inner: Mutex<Option<CachedRates>>,
}

struct CachedRates {
    rates: ExchangeRates,
    cached_at: Instant,
}

impl ExchangeRateCache {
    pub fn new() -> Self {
        ExchangeRateCache {
            inner: Mutex::new(None),
        }
    }

    pub fn get(&self) -> Option<ExchangeRates> {
        let lock = self.inner.lock().unwrap();
        if let Some(cached) = &*lock {
            if cached.cached_at.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
                return Some(cached.rates.clone());
            }
        }
        None
    }

    pub fn set(&self, rates: ExchangeRates) {
        let mut lock = self.inner.lock().unwrap();
        *lock = Some(CachedRates {
            rates,
            cached_at: Instant::now(),
        });
    }

    /// Returns cached rates even if stale (for offline fallback).
    pub fn get_stale(&self) -> Option<ExchangeRates> {
        let lock = self.inner.lock().unwrap();
        lock.as_ref().map(|c| c.rates.clone())
    }
}

/// Fetch the latest exchange rates from open.er-api.com (USD base).
pub async fn fetch_exchange_rates() -> Result<ExchangeRates, String> {
    let url = "https://open.er-api.com/v6/latest/USD";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("Network error fetching exchange rates: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Exchange rate API error: HTTP {}",
            response.status()
        ));
    }

    let data: ErApiResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse exchange rate response: {}", e))?;

    if let Some(result) = &data.result {
        if result != "success" {
            return Err(format!("Exchange rate API returned non-success result: {}", result));
        }
    }

    let rates = data
        .rates
        .ok_or_else(|| "Exchange rate API returned no rates".to_string())?;

    let usd_cny = *rates
        .get("CNY")
        .ok_or_else(|| "Missing CNY rate".to_string())?;
    let usd_hkd = *rates
        .get("HKD")
        .ok_or_else(|| "Missing HKD rate".to_string())?;
    let cny_hkd = usd_hkd / usd_cny;

    Ok(ExchangeRates {
        usd_cny,
        usd_hkd,
        cny_hkd,
        updated_at: Utc::now().to_rfc3339(),
    })
}

/// Get exchange rates, using the cache when fresh, and fetching if stale.
/// Falls back to stale cache on network errors.
pub async fn get_cached_rates(cache: &ExchangeRateCache) -> Result<ExchangeRates, String> {
    if let Some(rates) = cache.get() {
        return Ok(rates);
    }

    match fetch_exchange_rates().await {
        Ok(rates) => {
            cache.set(rates.clone());
            Ok(rates)
        }
        Err(e) => {
            // Offline fallback: return stale cache if available
            if let Some(stale) = cache.get_stale() {
                eprintln!("Warning: could not fetch fresh exchange rates ({}), using stale cache", e);
                Ok(stale)
            } else {
                Err(e)
            }
        }
    }
}

/// Convert an amount from one currency to another using the given rates.
/// Supported pairs: USD, CNY, HKD.
pub fn convert_currency(amount: f64, from: &str, to: &str, rates: &ExchangeRates) -> f64 {
    if from == to {
        return amount;
    }
    // Convert to USD first, then to target
    let usd_amount = to_usd(amount, from, rates);
    from_usd(usd_amount, to, rates)
}

fn to_usd(amount: f64, currency: &str, rates: &ExchangeRates) -> f64 {
    match currency {
        "USD" => amount,
        "CNY" => amount / rates.usd_cny,
        "HKD" => amount / rates.usd_hkd,
        _ => amount,
    }
}

fn from_usd(amount: f64, currency: &str, rates: &ExchangeRates) -> f64 {
    match currency {
        "USD" => amount,
        "CNY" => amount * rates.usd_cny,
        "HKD" => amount * rates.usd_hkd,
        _ => amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rates() -> ExchangeRates {
        ExchangeRates {
            usd_cny: 7.2,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.2,
            updated_at: "2024-01-15T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_convert_usd_to_cny() {
        let rates = sample_rates();
        let result = convert_currency(100.0, "USD", "CNY", &rates);
        assert!((result - 720.0).abs() < 0.001);
    }

    #[test]
    fn test_convert_cny_to_usd() {
        let rates = sample_rates();
        let result = convert_currency(720.0, "CNY", "USD", &rates);
        assert!((result - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_convert_same_currency() {
        let rates = sample_rates();
        let result = convert_currency(100.0, "USD", "USD", &rates);
        assert!((result - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_convert_usd_to_hkd() {
        let rates = sample_rates();
        let result = convert_currency(100.0, "USD", "HKD", &rates);
        assert!((result - 780.0).abs() < 0.001);
    }

    #[test]
    fn test_convert_cny_to_hkd() {
        let rates = sample_rates();
        let result = convert_currency(7.2, "CNY", "HKD", &rates);
        // 7.2 CNY = 1 USD = 7.8 HKD
        assert!((result - 7.8).abs() < 0.001);
    }

    #[test]
    fn test_exchange_rate_cache_empty() {
        let cache = ExchangeRateCache::new();
        assert!(cache.get().is_none());
        assert!(cache.get_stale().is_none());
    }

    #[test]
    fn test_exchange_rate_cache_set_and_get() {
        let cache = ExchangeRateCache::new();
        let rates = sample_rates();
        cache.set(rates.clone());
        let cached = cache.get().expect("should have cached rates");
        assert!((cached.usd_cny - 7.2).abs() < 0.001);
    }

    #[test]
    fn test_exchange_rate_cache_stale_fallback() {
        let cache = ExchangeRateCache::new();
        let rates = sample_rates();
        cache.set(rates);
        // Stale should always return if set
        let stale = cache.get_stale().expect("should have stale rates");
        assert!((stale.usd_cny - 7.2).abs() < 0.001);
    }
}
