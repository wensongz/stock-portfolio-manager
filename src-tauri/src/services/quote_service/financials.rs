use crate::models::FinancialReport;
use crate::services::http_client;

/// Strip a CN symbol's market prefix to get the bare 6-digit code.
/// `"sh600519"` → `"600519"`; `"600519"` → `"600519"`.
fn cn_bare_code(symbol: &str) -> String {
    let s = symbol.trim().to_ascii_lowercase();
    let is_six_digit_code =
        |value: &str| value.len() == 6 && value.chars().all(|character| character.is_ascii_digit());
    if let Some(code) = ["sh", "sz", "bj"].iter().find_map(|prefix| {
        s.strip_prefix(prefix)
            .filter(|value| is_six_digit_code(value))
    }) {
        return code.to_string();
    }
    if let Some(code) = [".ss", ".sz"].iter().find_map(|suffix| {
        s.strip_suffix(suffix)
            .filter(|value| is_six_digit_code(value))
    }) {
        return code.to_string();
    }
    s
}

/// Fetch recent financial-statement periods (最近 N 期财报) from East Money's
/// datacenter API.
///
/// Currently only supports CN A-shares (the datacenter `SECURITY_CODE` is the
/// 6-digit code). Returns the most recent `limit` periods (default 4), newest
/// first. No authentication is required.
pub async fn fetch_financial_statements(
    symbol: &str,
    market: &str,
    limit: usize,
) -> Result<Vec<FinancialReport>, String> {
    if market != "CN" {
        return Err(format!(
            "财务报表查询暂仅支持 A 股，{} 的市场为 {}",
            symbol, market
        ));
    }
    let code = cn_bare_code(symbol);
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("无效的 A 股代码用于财务报表查询: {}", symbol));
    }

    let columns = "REPORT_DATE,REPORT_DATE_NAME,EPSJB,ROEJQ,OPERATE_INCOME_PK,TOTALOPERATEREVETZ,\
PARENTNETPROFIT,PARENTNETPROFITTZ,TOTAL_ASSETS_PK,INTEREST_DEBT_RATIO";
    let url = format!(
        "https://datacenter.eastmoney.com/securities/api/data/v1/get?\
reportName=RPT_F10_FINANCE_MAINFINADATA&columns={}&filter=(SECURITY_CODE=\"{}\")\
&pageSize={}&sortColumns=REPORT_DATE&sortTypes=-1",
        columns, code, limit
    );

    let resp = http_client::eastmoney_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("查询东方财富财务报表失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "fetch_financial_statements: HTTP {} for {}",
            resp.status(),
            symbol
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("fetch_financial_statements: 解析失败 for {}: {}", symbol, e))?;

    if !body
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(format!("东方财富财务报表查询失败 for {}: {}", symbol, msg));
    }

    let rows = body["result"]["data"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let parse_date = |s: &str| -> String {
        // "2026-03-31 00:00:00" -> "2026-03-31"
        s.split_whitespace().next().unwrap_or(s).to_string()
    };
    let as_f64 = |v: &serde_json::Value| -> Option<f64> {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    };

    let reports = rows
        .iter()
        .map(|r| {
            let report_date = r
                .get("REPORT_DATE")
                .and_then(|v| v.as_str())
                .map(parse_date)
                .unwrap_or_default();
            FinancialReport {
                period_name: r
                    .get("REPORT_DATE_NAME")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                report_date,
                eps: r.get("EPSJB").and_then(as_f64),
                roe: r.get("ROEJQ").and_then(as_f64),
                revenue: r.get("OPERATE_INCOME_PK").and_then(as_f64),
                revenue_yoy: r.get("TOTALOPERATEREVETZ").and_then(as_f64),
                net_profit: r.get("PARENTNETPROFIT").and_then(as_f64),
                net_profit_yoy: r.get("PARENTNETPROFITTZ").and_then(as_f64),
                total_assets: r.get("TOTAL_ASSETS_PK").and_then(as_f64),
                debt_ratio: r.get("INTEREST_DEBT_RATIO").and_then(as_f64),
            }
        })
        .collect();
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::cn_bare_code;

    #[test]
    fn cn_bare_code_accepts_exchange_prefixes_and_yahoo_suffixes() {
        assert_eq!(cn_bare_code("sh600519"), "600519");
        assert_eq!(cn_bare_code("SZ000001"), "000001");
        assert_eq!(cn_bare_code("bj920001"), "920001");
        assert_eq!(cn_bare_code("600519.SS"), "600519");
        assert_eq!(cn_bare_code("000001.sz"), "000001");
    }
}
