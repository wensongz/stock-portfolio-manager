use crate::services::http_client;
use serde::{Deserialize, Serialize};
use std::io::Write;

/// One parsed trade row extracted from a 同花顺 (THS) screenshot.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParsedTradeRow {
    /// "BUY" or "SELL"
    pub transaction_type: String,
    /// Stock name as recognised by OCR (e.g. "贵州茅台")
    pub stock_name: String,
    /// ISO-8601 datetime string combining date + time found in the screenshot,
    /// e.g. "2026-04-03T09:30:00"
    pub traded_at: String,
    /// Per-share price
    pub price: f64,
    /// Number of shares
    pub shares: f64,
    /// Transaction total (price × shares before commission)
    pub total_amount: f64,
    /// Commission / stamp-duty paid
    pub commission: f64,
}

// ---------------------------------------------------------------------------
// Xueqiu stock-name → A-share code lookup
// ---------------------------------------------------------------------------

/// Response structure returned by Xueqiu stock search API.
#[derive(Debug, Deserialize)]
struct XueqiuSearchResponse {
    data: Option<XueqiuSearchData>,
}

#[derive(Debug, Deserialize)]
struct XueqiuSearchData {
    items: Option<Vec<XueqiuSearchItem>>,
}

#[derive(Debug, Deserialize)]
struct XueqiuSearchItem {
    /// e.g. "SH600036", "SZ000001"
    symbol: Option<String>,
    /// e.g. "CN"
    #[serde(rename = "type")]
    stock_type: Option<String>,
}

/// Query Xueqiu to resolve a Chinese stock name to its 6-digit A-share code.
///
/// Returns `Ok(Some("600036"))` on success, `Ok(None)` when no CN result is
/// found, and `Err(…)` for network / API failures.
#[tauri::command(rename_all = "camelCase")]
pub async fn lookup_cn_stock_code(name: String) -> Result<Option<String>, String> {
    use std::time::Duration;

    // Ensure Xueqiu session is initialised (reuse the existing helper via a
    // minimal ad-hoc approach: just call the homepage once if needed).
    // We deliberately avoid importing private quote_service internals and
    // instead build the request directly on the shared xueqiu_client.
    let client = http_client::xueqiu_client();

    // A minimal session warm-up: if the client has no cookie yet, visit the
    // homepage so Xueqiu sets xq_a_token.  We use a static AtomicBool to
    // perform this only once per process.
    static INIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !INIT.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = client
            .get("https://xueqiu.com")
            .header(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        INIT.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    let url = format!(
        "https://xueqiu.com/stock/search.json?q={}&type=1&count=5",
        urlencoding::encode(&name)
    );

    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("查询雪球失败: {}", e))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: XueqiuSearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析雪球响应失败: {}", e))?;

    let items = body
        .data
        .and_then(|d| d.items)
        .unwrap_or_default();

    // Find first CN A-share result: symbol starts with "SH" or "SZ" and has
    // exactly 8 chars (prefix 2 + code 6).
    for item in &items {
        let sym = match &item.symbol {
            Some(s) if !s.is_empty() => s.as_str(),
            _ => continue,
        };
        // stock_type field may say "stock" for A-shares; the symbol prefix is
        // the most reliable indicator.
        let is_cn = sym.starts_with("SH") || sym.starts_with("SZ");
        let _ = &item.stock_type; // kept for potential future filtering
        if is_cn && sym.len() == 8 {
            return Ok(Some(sym[2..].to_string()));
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// OCR parsing
// ---------------------------------------------------------------------------

/// Run Tesseract on raw image bytes and return the recognised text.
///
/// `data` is the raw PNG/JPEG file content.  Tesseract is invoked via
/// `std::process::Command` so no native library linking is required.
fn ocr_image(data: &[u8]) -> Result<String, String> {
    // Write image bytes to a temp file.
    let mut tmp = tempfile::Builder::new()
        .suffix(".png")
        .tempfile()
        .map_err(|e| format!("创建临时文件失败: {}", e))?;
    tmp.write_all(data)
        .map_err(|e| format!("写临时文件失败: {}", e))?;
    tmp.flush()
        .map_err(|e| format!("刷新临时文件失败: {}", e))?;

    let input_path = tmp.path().to_owned();

    // Create a second temp file for the output txt (tesseract appends .txt).
    let out_tmp = tempfile::Builder::new()
        .suffix(".txt")
        .tempfile()
        .map_err(|e| format!("创建输出临时文件失败: {}", e))?;
    // Drop the file handle so tesseract can write to it; keep the path.
    let out_base = out_tmp
        .path()
        .to_str()
        .ok_or("输出路径无效")?
        .trim_end_matches(".txt")
        .to_string();
    drop(out_tmp);

    let output = std::process::Command::new("tesseract")
        .arg(&input_path)
        .arg(&out_base)
        .arg("-l")
        .arg("chi_sim")
        .arg("--psm")
        .arg("6")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "未找到 tesseract 可执行文件。请安装 Tesseract-OCR 和中文语言包（chi_sim）后重试。\n\
                 macOS: brew install tesseract tesseract-lang\n\
                 Ubuntu: sudo apt install tesseract-ocr tesseract-ocr-chi-sim"
                    .to_string()
            } else {
                format!("启动 tesseract 失败: {}", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Tesseract 执行失败: {}", stderr));
    }

    // Read the output .txt file.
    let out_file = format!("{}.txt", out_base);
    std::fs::read_to_string(&out_file)
        .map_err(|e| format!("读取 OCR 结果失败: {}", e))
}

/// Parse the plain-text output of Tesseract (from a 同花顺 trade screenshot)
/// into a list of [`ParsedTradeRow`] values.
///
/// # 同花顺 layout
///
/// The app groups transactions by month. Each month starts with a header like
/// `2026-04` followed by individual trade entries. Every trade entry spans
/// multiple OCR lines in roughly the following structure:
///
/// ```text
/// 买入-贵州茅台
/// 04-03  09:30   1505.00  100  150500.00  5.00
/// ```
///
/// or split across lines when columns wrap:
///
/// ```text
/// 买入-贵州茅台
/// 04-03  09:30
/// 1505.00  100  150500.00  5.00
/// ```
///
/// We therefore:
/// 1. Extract the current year from the first `YYYY-MM` month header found.
/// 2. Walk lines top-to-bottom looking for lines that start with `买入` or `卖出`.
/// 3. Collect subsequent lines until we have assembled: date+time, price,
///    shares, total_amount, commission.
fn parse_ths_ocr(text: &str) -> Vec<ParsedTradeRow> {
    // Step 1 – find year from month header (e.g. "2026-04")
    let year = extract_year(text);

    let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    let mut rows: Vec<ParsedTradeRow> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Look for a trade header line: starts with 买入 or 卖出 followed by
        // optional separator (dash, en-dash, em-dash, space, colon) and name.
        if let Some((tx_type, stock_name)) = parse_trade_header(line) {
            // Collect up to 4 subsequent non-empty lines to find the fields.
            let mut window: Vec<&str> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() && window.len() < 5 {
                let l = lines[j].trim();
                if !l.is_empty() {
                    window.push(l);
                }
                j += 1;
            }

            // Try to extract date+time and numeric fields from the window.
            if let Some(row) =
                extract_trade_fields(&tx_type, &stock_name, year, &window)
            {
                rows.push(row);
            }

            // Advance past the consumed lines.
            i = j;
        } else {
            i += 1;
        }
    }

    // Sort chronologically so that create_transaction is called oldest-first.
    rows.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));
    rows
}

/// Return the 4-digit year found in the first `YYYY-MM` header in `text`.
/// Falls back to the current UTC year if none is found.
fn extract_year(text: &str) -> i32 {
    // Match YYYY-MM at the start of a word (possibly with surrounding spaces)
    let re = regex::Regex::new(r"(?m)^\s*(\d{4})-\d{2}\s*$").unwrap();
    if let Some(cap) = re.captures(text) {
        if let Ok(y) = cap[1].parse::<i32>() {
            return y;
        }
    }
    // Also search inline
    let re2 = regex::Regex::new(r"\b(\d{4})-\d{2}\b").unwrap();
    if let Some(cap) = re2.captures(text) {
        if let Ok(y) = cap[1].parse::<i32>() {
            return y;
        }
    }
    chrono::Utc::now().format("%Y").to_string().parse().unwrap_or(2025)
}

/// Detect a 买入 / 卖出 header line and return `(transaction_type, stock_name)`.
///
/// Accepted separators between verb and name: `-` `–` `—` ` ` `:` `：`.
fn parse_trade_header(line: &str) -> Option<(String, String)> {
    // The line may be "买入-贵州茅台" or "卖出 贵州茅台" etc.
    let (tx_type, rest) = if line.starts_with("买入") {
        ("BUY", &line["买入".len()..])
    } else if line.starts_with("卖出") {
        ("SELL", &line["卖出".len()..])
    } else {
        return None;
    };

    // Strip leading separator characters.
    let name = rest
        .trim_start_matches(['-', '–', '—', ' ', ':', '：', '\u{2013}', '\u{2014}'])
        .trim();

    if name.is_empty() {
        return None;
    }

    Some((tx_type.to_string(), name.to_string()))
}

/// Extract numeric fields and date/time from lines following a trade header.
///
/// Searches for a line containing both `MM-DD` and `HH:MM` patterns, then
/// collects the first four positive decimal numbers found in the remaining
/// text as price, shares, total_amount, commission.
fn extract_trade_fields(
    tx_type: &str,
    stock_name: &str,
    year: i32,
    window: &[&str],
) -> Option<ParsedTradeRow> {
    // Regex patterns
    let date_re = regex::Regex::new(r"\b(\d{1,2})-(\d{2})\b").unwrap();
    let time_re = regex::Regex::new(r"\b(\d{1,2}):(\d{2})\b").unwrap();
    let num_re = regex::Regex::new(r"\b(\d+(?:\.\d+)?)\b").unwrap();

    let all_text = window.join(" ");

    // Locate date and time.
    let (month, day) = date_re
        .captures(&all_text)
        .map(|c| {
            (
                c[1].parse::<u32>().unwrap_or(1),
                c[2].parse::<u32>().unwrap_or(1),
            )
        })?;

    let (hour, minute) = time_re
        .captures(&all_text)
        .map(|c| {
            (
                c[1].parse::<u32>().unwrap_or(9),
                c[2].parse::<u32>().unwrap_or(30),
            )
        })
        .unwrap_or((9, 30));

    let traded_at = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:00",
        year, month, day, hour, minute
    );

    // Collect all decimal numbers from the window, excluding date/time parts.
    // We remove the date (MM-DD) and time (HH:MM) tokens first to avoid
    // picking up month/day/hour/minute as numbers.
    let cleaned = date_re.replace_all(&all_text, " ");
    let cleaned = time_re.replace_all(&cleaned, " ");

    let numbers: Vec<f64> = num_re
        .captures_iter(&cleaned)
        .filter_map(|c| c[1].parse::<f64>().ok())
        .filter(|&n| n > 0.0)
        .collect();

    // We need at least 4 numbers: price, shares, total_amount, commission.
    // In some layouts there may be extra numbers (e.g. sequence numbers).
    // The most reliable heuristic: the first large-ish number is price,
    // then shares (integer), then total_amount (price × shares ≈), then
    // commission (smallest, last).
    //
    // Find price, shares, total_amount, commission by trying combinations.
    let (price, shares, total_amount, commission) = pick_fields(&numbers)?;

    Some(ParsedTradeRow {
        transaction_type: tx_type.to_string(),
        stock_name: stock_name.to_string(),
        traded_at,
        price,
        shares,
        total_amount,
        commission,
    })
}

/// Pick price/shares/total/commission from a list of numbers extracted from
/// the OCR text.
///
/// Strategy: iterate over candidate tuples (price, shares, total, commission)
/// and return the first one where `|price * shares - total| / total < 1%`.
/// Falls back to returning the first four numbers if no consistent tuple is found.
fn pick_fields(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if numbers.len() < 4 {
        return None;
    }

    // Try every combination of 4 numbers where total ≈ price × shares.
    let n = numbers.len().min(8); // limit search space
    for i in 0..n {
        for j in 0..n {
            if j == i {
                continue;
            }
            let price = numbers[i];
            let shares = numbers[j];
            if shares < 1.0 || price <= 0.0 {
                continue;
            }
            let expected_total = price * shares;
            for k in 0..n {
                if k == i || k == j {
                    continue;
                }
                let total = numbers[k];
                if total <= 0.0 {
                    continue;
                }
                let rel_err = (expected_total - total).abs() / total;
                if rel_err < 0.02 {
                    // good match – find commission as any remaining number
                    let commission = numbers
                        .iter()
                        .enumerate()
                        .filter(|(idx, _)| *idx != i && *idx != j && *idx != k)
                        .map(|(_, &v)| v)
                        .find(|&v| v >= 0.0)
                        .unwrap_or(0.0);
                    return Some((price, shares, total, commission));
                }
            }
        }
    }

    // Fallback: return first four numbers as-is.
    Some((numbers[0], numbers[1], numbers[2], numbers[3]))
}

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

/// Decode a base64-encoded image, run Tesseract OCR on it, and return the
/// parsed trade rows.
///
/// The caller should pass `image_base64` as a pure base64 string (no
/// `data:image/...;base64,` prefix, though the prefix is stripped if present).
#[tauri::command(rename_all = "camelCase")]
pub async fn parse_trade_image(image_base64: String) -> Result<Vec<ParsedTradeRow>, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // Strip optional data-URL prefix.
    let b64 = if let Some(pos) = image_base64.find("base64,") {
        &image_base64[pos + "base64,".len()..]
    } else {
        &image_base64
    };

    let bytes = STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("base64 解码失败: {}", e))?;

    let text = ocr_image(&bytes)?;
    let rows = parse_ths_ocr(&text);
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_year_from_header() {
        assert_eq!(extract_year("2026-04"), 2026);
        assert_eq!(extract_year("foo\n2025-12\nbar"), 2025);
    }

    #[test]
    fn test_extract_year_fallback() {
        let y = extract_year("no year here");
        assert!(y >= 2024); // current year
    }

    #[test]
    fn test_parse_trade_header_buy() {
        let (tx, name) = parse_trade_header("买入-贵州茅台").unwrap();
        assert_eq!(tx, "BUY");
        assert_eq!(name, "贵州茅台");
    }

    #[test]
    fn test_parse_trade_header_sell_space() {
        let (tx, name) = parse_trade_header("卖出 招商银行").unwrap();
        assert_eq!(tx, "SELL");
        assert_eq!(name, "招商银行");
    }

    #[test]
    fn test_parse_trade_header_none() {
        assert!(parse_trade_header("2026-04").is_none());
        assert!(parse_trade_header("普通文本").is_none());
    }

    #[test]
    fn test_pick_fields_basic() {
        // price=1505.00, shares=100, total=150500.00, commission=5.00
        let nums = vec![1505.0f64, 100.0, 150500.0, 5.0];
        let (price, shares, total, comm) = pick_fields(&nums).unwrap();
        assert!((price - 1505.0).abs() < 0.01);
        assert!((shares - 100.0).abs() < 0.01);
        assert!((total - 150500.0).abs() < 1.0);
        assert!((comm - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_ths_ocr_single_trade() {
        let text = "2026-04\n买入-贵州茅台\n04-03  09:30   1505.00  100  150500.00  5.00\n";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.transaction_type, "BUY");
        assert_eq!(r.stock_name, "贵州茅台");
        assert_eq!(r.traded_at, "2026-04-03T09:30:00");
        assert!((r.price - 1505.0).abs() < 0.01);
        assert!((r.shares - 100.0).abs() < 0.01);
        assert!((r.total_amount - 150500.0).abs() < 1.0);
        assert!((r.commission - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_ths_ocr_sell() {
        let text = "2026-04\n卖出-招商银行\n04-10  14:55   38.50  500  19250.00  3.00\n";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].transaction_type, "SELL");
        assert_eq!(rows[0].stock_name, "招商银行");
    }

    #[test]
    fn test_parse_ths_ocr_multiple_trades_sorted() {
        let text = "\
2026-04
买入-贵州茅台
04-10  10:00   1505.00  100  150500.00  5.00
卖出-招商银行
04-03  14:00   38.50  500  19250.00  3.00
";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 2);
        // Should be sorted by traded_at: 04-03 before 04-10
        assert!(rows[0].traded_at < rows[1].traded_at);
    }
}
