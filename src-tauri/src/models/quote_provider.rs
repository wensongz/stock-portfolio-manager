use serde::{Deserialize, Serialize};

/// Configuration for which quote data provider to use per market.
///
/// Supported providers:
/// - `"yahoo"`     – Yahoo Finance, supports HK / US
/// - `"eastmoney"` – East Money (东方财富), supports CN / HK / US
/// - `"xueqiu"`    – Xueqiu (雪球), supports CN / HK / US
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteProviderConfig {
    /// Provider for US stocks: "xueqiu" (default)
    pub us_provider: String,
    /// Provider for HK stocks: "xueqiu" (default)
    pub hk_provider: String,
    /// Provider for CN A-shares: "xueqiu" (default)
    pub cn_provider: String,
    /// Optional user-provided Xueqiu cookie string (e.g. `xq_a_token=xxx`).
    /// When set, this replaces the auto-obtained `xq_a_token` in API requests.
    #[serde(default)]
    pub xueqiu_cookie: Option<String>,
    /// Xueqiu `u` cookie value (user ID from a logged-in browser session).
    /// The kline API requires both `xq_a_token` and `u` to return data.
    #[serde(default)]
    pub xueqiu_u: Option<String>,

    /// Whether SELL and PAY (dividend) transactions adjust the A-share cost basis.
    /// Defaults to `true` following mainland China brokerage conventions.
    #[serde(default = "default_cn_adjust")]
    pub cn_adjust_sell_pay_cost: bool,
    /// Whether SELL and PAY (dividend) transactions adjust the US-stock cost basis.
    /// Defaults to `false` (IB-style: realised gains are taxed; cost basis is fixed).
    #[serde(default)]
    pub us_adjust_sell_pay_cost: bool,
    /// Whether SELL and PAY (dividend) transactions adjust the HK-stock cost basis.
    /// Defaults to `false` (same reasoning as US stocks).
    #[serde(default)]
    pub hk_adjust_sell_pay_cost: bool,
}

fn default_cn_adjust() -> bool { true }

impl Default for QuoteProviderConfig {
    fn default() -> Self {
        QuoteProviderConfig {
            us_provider: "xueqiu".to_string(),
            hk_provider: "xueqiu".to_string(),
            cn_provider: "xueqiu".to_string(),
            xueqiu_cookie: None,
            xueqiu_u: None,
            cn_adjust_sell_pay_cost: true,
            us_adjust_sell_pay_cost: false,
            hk_adjust_sell_pay_cost: false,
        }
    }
}
