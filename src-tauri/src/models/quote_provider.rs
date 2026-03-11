use serde::{Deserialize, Serialize};

/// Configuration for which quote data provider to use per market.
///
/// Supported providers:
/// - `"yahoo"`     – Yahoo Finance, supports HK / US
/// - `"eastmoney"` – East Money (东方财富), supports CN / HK / US
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteProviderConfig {
    /// Provider for US stocks: "eastmoney" (default)
    pub us_provider: String,
    /// Provider for HK stocks: "eastmoney" (default)
    pub hk_provider: String,
    /// Provider for CN A-shares: "eastmoney" (default)
    pub cn_provider: String,
}

impl Default for QuoteProviderConfig {
    fn default() -> Self {
        QuoteProviderConfig {
            us_provider: "eastmoney".to_string(),
            hk_provider: "eastmoney".to_string(),
            cn_provider: "eastmoney".to_string(),
        }
    }
}
