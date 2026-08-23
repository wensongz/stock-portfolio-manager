mod everbright;
mod hsbc;

pub use everbright::convert_everbright;
pub use hsbc::convert_hsbc;

use regex::Regex;
use serde::Serialize;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub(super) struct BrokerTransactionRow {
    pub traded_at: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub transaction_type: String,
    pub shares: f64,
    pub price: f64,
    pub total_amount: Option<f64>,
    pub commission: f64,
    pub currency: String,
    pub notes: String,
    #[serde(skip)]
    pub source_order: usize,
}

pub(super) fn round(value: f64, digits: i32) -> f64 {
    let factor = 10_f64.powi(digits);
    (value * factor).round() / factor
}

pub(super) fn rows_to_csv(rows: &[BrokerTransactionRow]) -> Result<String, String> {
    let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());
    for row in rows {
        writer.serialize(row).map_err(|e| e.to_string())?;
    }
    let bytes = writer.into_inner().map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn natural_cmp(a: &Path, b: &Path) -> Ordering {
    let a = a.file_name().and_then(|v| v.to_str()).unwrap_or_default();
    let b = b.file_name().and_then(|v| v.to_str()).unwrap_or_default();
    let token_re = Regex::new(r"\d+|\D+").unwrap();
    let at: Vec<&str> = token_re.find_iter(a).map(|m| m.as_str()).collect();
    let bt: Vec<&str> = token_re.find_iter(b).map(|m| m.as_str()).collect();
    for (left, right) in at.iter().zip(bt.iter()) {
        let order = match (left.parse::<u64>(), right.parse::<u64>()) {
            (Ok(l), Ok(r)) => l.cmp(&r),
            _ => left.to_lowercase().cmp(&right.to_lowercase()),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    at.len().cmp(&bt.len())
}

pub(super) fn checked_paths(paths: Vec<String>, extension: &str) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    for value in paths {
        let path = PathBuf::from(&value);
        if !path.is_file() {
            return Err(format!("文件不存在：{}", path.display()));
        }
        let actual = path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        if !actual.eq_ignore_ascii_case(extension) {
            return Err(format!("不支持的文件类型：{}", path.display()));
        }
        result.push(path);
    }
    result.sort_by(|a, b| natural_cmp(a, b));
    Ok(result)
}

pub(super) fn validate_rows(
    rows: &[BrokerTransactionRow],
    expected_currency: Option<&str>,
) -> Result<(), String> {
    if rows.is_empty() {
        return Err("没有从所选文件中识别到可导入交易".to_string());
    }
    let date_re = Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    let cn_re = Regex::new(r"^(sh|sz)\d{6}$").unwrap();
    let hk_re = Regex::new(r"^\d{5}$").unwrap();
    for (index, row) in rows.iter().enumerate() {
        let line = index + 2;
        if !date_re.is_match(&row.traded_at) {
            return Err(format!("第 {} 行日期无效", line));
        }
        if !["BUY", "SELL", "PAY", "TRANSFER_IN", "TRANSFER_OUT"]
            .contains(&row.transaction_type.as_str())
        {
            return Err(format!("第 {} 行交易类型无效", line));
        }
        if row.market == "CN" && !cn_re.is_match(&row.symbol) {
            return Err(format!("第 {} 行 A 股代码缺少 sh/sz 前缀", line));
        }
        if row.market == "HK" && !hk_re.is_match(&row.symbol) {
            return Err(format!("第 {} 行港股代码不是五位数字", line));
        }
        if row.name.is_empty() {
            return Err(format!("第 {} 行缺少证券名称", line));
        }
        if row.transaction_type != "PAY" && (row.shares <= 0.0 || row.price < 0.0) {
            return Err(format!("第 {} 行成交数量或价格异常", line));
        }
        if let Some(currency) = expected_currency {
            if row.currency != currency {
                return Err(format!("第 {} 行币种异常", line));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_file_sort_handles_numeric_suffixes() {
        let mut paths = vec![PathBuf::from("ptzh10.xls"), PathBuf::from("ptzh2.xls")];
        paths.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(paths[0], PathBuf::from("ptzh2.xls"));
    }
}
