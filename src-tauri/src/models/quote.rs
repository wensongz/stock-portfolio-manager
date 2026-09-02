use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct StockQuote {
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub current_price: f64,
    pub previous_close: f64,
    pub change: f64,
    pub change_percent: f64,
    pub high: f64,
    pub low: f64,
    pub volume: i64,
    pub updated_at: String,
    // ── Fundamental / valuation snapshot (optional, may be absent) ──
    /// Trailing-twelve-month P/E ratio.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pe_ttm: Option<f64>,
    /// Price-to-book ratio.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pb: Option<f64>,
    /// Total market capitalisation in the quote's native currency.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub market_cap: Option<f64>,
    /// Dividend yield (percent).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub dividend_yield: Option<f64>,
    /// Earnings per share (TTM).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub eps: Option<f64>,
    /// Return on equity (percent).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub roe: Option<f64>,
    /// Turnover rate (percent).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub turnover_rate: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HoldingWithQuote {
    pub id: String,
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub category_id: Option<String>,
    pub shares: f64,
    pub avg_cost: f64,
    pub currency: String,
    pub created_at: String,
    pub updated_at: String,
    pub quote: Option<StockQuote>,
    pub market_value: Option<f64>,
    pub total_cost: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    pub unrealized_pnl_percent: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExchangeRates {
    pub usd_cny: f64,
    pub usd_hkd: f64,
    pub cny_hkd: f64,
    pub updated_at: String,
}

#[cfg(test)]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DailyPortfolioValue {
    pub id: i64,
    pub date: String,
    pub total_cost: f64,
    pub total_value: f64,
    pub us_cost: f64,
    pub us_value: f64,
    pub cn_cost: f64,
    pub cn_value: f64,
    pub hk_cost: f64,
    pub hk_value: f64,
    pub exchange_rates: String,
    pub daily_pnl: f64,
    pub cumulative_pnl: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DailyHoldingSnapshot {
    pub id: i64,
    pub date: String,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub category_name: Option<String>,
    pub shares: f64,
    pub avg_cost: f64,
    pub close_price: f64,
    pub market_value: f64,
}

/// A single OHLCV candlestick for one trading day.
///
/// Used by [`crate::services::indicators`] to compute technical indicators
/// (MA, MACD, RSI, Bollinger bands).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PriceCandle {
    /// ISO date, e.g. "2026-07-22".
    pub date: String,
    pub open: f64,
    pub close: f64,
    pub high: f64,
    pub low: f64,
    /// Trading volume.
    pub volume: f64,
}

/// One period of financial-statement data for fundamental analysis.
///
/// Populated from East Money's `datacenter` financial API
/// (`RPT_F10_FINANCE_MAINFINADATA`). Amounts are in the quote's native
/// currency (CNY for A-shares). `*_yoy` fields are year-over-year growth in
/// **percent** (e.g. `6.34` = +6.34% YoY); `None` when the source did not
/// report a value.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FinancialReport {
    /// Report period, e.g. "2026一季报" / "2025年报".
    pub period_name: String,
    /// ISO date of the report period end, e.g. "2026-03-31".
    pub report_date: String,
    /// Basic earnings per share.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eps: Option<f64>,
    /// Weighted return on equity (percent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roe: Option<f64>,
    /// Total operating revenue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<f64>,
    /// Revenue year-over-year growth (percent; 6.34 = +6.34%).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue_yoy: Option<f64>,
    /// Net profit attributable to parent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_profit: Option<f64>,
    /// Net-profit year-over-year growth (percent; 6.34 = +6.34%).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_profit_yoy: Option<f64>,
    /// Total assets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_assets: Option<f64>,
    /// Asset-liability ratio (percent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debt_ratio: Option<f64>,
}
