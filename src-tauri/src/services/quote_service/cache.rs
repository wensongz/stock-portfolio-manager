use super::{fetch_quotes_batch_with_providers, QuoteFetchResult, QuoteServiceState};
use crate::models::StockQuote;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

struct CachedQuote {
    quote: StockQuote,
    _cached_at: Instant,
}

/// In-memory cache for stock quotes, keyed by symbol.
pub struct QuoteCache {
    inner: Mutex<HashMap<String, CachedQuote>>,
}

impl QuoteCache {
    pub fn new() -> Self {
        QuoteCache {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Returns a cached quote if it exists (no TTL – the cache is only
    /// refreshed when the caller explicitly requests it).
    pub fn get(&self, symbol: &str) -> Option<StockQuote> {
        let lock = self.inner.lock().unwrap();
        lock.get(symbol).map(|c| c.quote.clone())
    }

    /// Returns a cached quote even if stale (for offline fallback).
    pub fn get_stale(&self, symbol: &str) -> Option<StockQuote> {
        let lock = self.inner.lock().unwrap();
        lock.get(symbol).map(|c| c.quote.clone())
    }

    /// Cache a single quote.
    pub fn set(&self, quote: StockQuote) {
        let mut lock = self.inner.lock().unwrap();
        lock.insert(
            quote.symbol.clone(),
            CachedQuote {
                quote,
                _cached_at: Instant::now(),
            },
        );
    }

    /// Cache multiple quotes at once.
    pub fn set_batch(&self, quotes: &[StockQuote]) {
        let mut lock = self.inner.lock().unwrap();
        let now = Instant::now();
        for q in quotes {
            lock.insert(
                q.symbol.clone(),
                CachedQuote {
                    quote: q.clone(),
                    _cached_at: now,
                },
            );
        }
    }

    /// Merge lightweight realtime quotes with richer cached metadata, then
    /// replace the cached prices. The realtime endpoint intentionally omits
    /// fields such as company name, P/E and dividend yield.
    pub fn merge_and_set_batch(&self, quotes: &mut [StockQuote]) {
        let mut lock = self.inner.lock().unwrap();
        let now = Instant::now();
        for quote in quotes {
            if let Some(cached) = lock.get(&quote.symbol).map(|entry| &entry.quote) {
                if quote.name.trim().is_empty() || quote.name == quote.symbol {
                    quote.name = cached.name.clone();
                }
                quote.pe_ttm = quote.pe_ttm.or(cached.pe_ttm);
                quote.pb = quote.pb.or(cached.pb);
                quote.market_cap = quote.market_cap.or(cached.market_cap);
                quote.dividend_yield = quote.dividend_yield.or(cached.dividend_yield);
                quote.eps = quote.eps.or(cached.eps);
                quote.roe = quote.roe.or(cached.roe);
                quote.turnover_rate = quote.turnover_rate.or(cached.turnover_rate);
            }
            lock.insert(
                quote.symbol.clone(),
                CachedQuote {
                    quote: quote.clone(),
                    _cached_at: now,
                },
            );
        }
    }

    /// Returns all cached quotes for the given symbols, plus the list of
    /// symbols that are missing from the cache.
    pub fn get_batch(
        &self,
        symbols: &[(String, String)],
    ) -> (Vec<StockQuote>, Vec<(String, String)>) {
        let lock = self.inner.lock().unwrap();
        let mut cached = Vec::new();
        let mut missing = Vec::new();
        for (symbol, market) in symbols {
            if let Some(entry) = lock.get(symbol.as_str()) {
                cached.push(entry.quote.clone());
            } else {
                missing.push((symbol.clone(), market.clone()));
            }
        }
        (cached, missing)
    }

    /// Drop every cached quote. Used by `factory_reset` so the in-memory
    /// cache does not keep serving prices that no longer correspond to any
    /// holding after the database is wiped.
    pub fn clear(&self) {
        let mut lock = self.inner.lock().unwrap();
        lock.clear();
    }
}

/// Deduplicate a list of (symbol, market) pairs, keeping only the first
/// occurrence of each symbol.  This avoids redundant API calls when the same
/// stock is held in multiple accounts.
pub(super) fn deduplicate_symbols(symbols: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    symbols
        .into_iter()
        .filter(|(symbol, _)| seen.insert(symbol.clone()))
        .collect()
}

/// Batch fetch quotes using the cache with specified providers.
/// Duplicate symbols are automatically deduplicated so that each symbol is
/// looked up and fetched only once, even when held in multiple accounts.
/// When `force_refresh` is true the cache is bypassed and all symbols are
/// fetched from the upstream API.
pub async fn fetch_quotes_batch_cached_with_providers(
    state: &QuoteServiceState,
    cache: &QuoteCache,
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
    cn_provider: &str,
    force_refresh: bool,
) -> Result<QuoteFetchResult<Vec<StockQuote>>, String> {
    // Deduplicate symbols so we only look up / fetch each symbol once.
    let unique_symbols = deduplicate_symbols(symbols);

    if force_refresh {
        // Force refresh: fetch all symbols from the upstream API.
        let mut fresh = fetch_quotes_batch_with_providers(
            state,
            unique_symbols.clone(),
            us_provider,
            hk_provider,
            cn_provider,
        )
        .await?;
        cache.merge_and_set_batch(&mut fresh.data);

        // Fall back to stale cache for any symbols that failed to fetch
        let fetched_symbols: std::collections::HashSet<String> =
            fresh.data.iter().map(|q| q.symbol.clone()).collect();
        let mut result = fresh.data;
        for (symbol, _) in &unique_symbols {
            if !fetched_symbols.contains(symbol) {
                if let Some(stale) = cache.get_stale(symbol) {
                    result.push(stale);
                }
            }
        }
        return Ok(QuoteFetchResult {
            data: result,
            warning: fresh.warning,
            did_refresh: fresh.did_refresh,
        });
    }

    let (mut result, missing) = cache.get_batch(&unique_symbols);

    if missing.is_empty() {
        return Ok(QuoteFetchResult {
            data: result,
            warning: None,
            did_refresh: false,
        });
    }

    let mut fresh = fetch_quotes_batch_with_providers(
        state,
        missing.clone(),
        us_provider,
        hk_provider,
        cn_provider,
    )
    .await?;
    cache.merge_and_set_batch(&mut fresh.data);
    result.extend(fresh.data);

    // For any symbols that were missing from fresh results (fetch failed),
    // try to use stale cache as fallback
    let fetched_symbols: std::collections::HashSet<String> =
        result.iter().map(|q| q.symbol.clone()).collect();
    for (symbol, _) in &missing {
        if !fetched_symbols.contains(symbol) {
            if let Some(stale) = cache.get_stale(symbol) {
                result.push(stale);
            }
        }
    }

    Ok(QuoteFetchResult {
        data: result,
        warning: fresh.warning,
        did_refresh: fresh.did_refresh,
    })
}
