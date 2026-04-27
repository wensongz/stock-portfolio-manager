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
/// THS displays one trade per "card" or row. After OCR the text may look like:
///
/// ```text
/// 2026-04
/// 卖出双汇发展          2026-04-09 09:58
/// 28.41  2000  56820.00  33.98
/// 买入-招商银行
/// 04-22 14:26   28.95   2000   57900.00   150.00
/// ```
///
/// Key observations:
/// - "买入"/"卖出" may appear **anywhere** on a line (not just at the start).
/// - The stock name consists purely of CJK characters (stop at digits/ASCII).
/// - THS sometimes shows a signed P&L amount (e.g. -56786.02) — these must be
///   discarded before numeric field extraction.
/// - Column order in the data lines is always:  price → shares → total → commission.
///
/// Algorithm:
/// 1. Extract the year from the first `YYYY-MM` header.
/// 2. Find every line containing "买入" or "卖出".
/// 3. For each anchor: extract CJK stock name; collect remaining text on the
///    same line plus up to 6 subsequent non-anchor lines as context.
/// 4. Remove negative numbers, percentages, dates, and times; then assign
///    numeric fields using an ordered positional heuristic.
fn parse_ths_ocr(text: &str) -> Vec<ParsedTradeRow> {
    let year = extract_year(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut rows: Vec<ParsedTradeRow> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if let Some((tx_type, stock_name, anchor_extra)) = detect_trade_anchor(line) {
            // Collect up to 6 subsequent non-empty, non-anchor lines as context.
            let mut window: Vec<&str> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() && window.len() < 6 {
                let l = lines[j].trim();
                if !l.is_empty() {
                    // Stop if we hit the next trade anchor.
                    if is_trade_anchor(l) {
                        break;
                    }
                    window.push(l);
                }
                j += 1;
            }

            if let Some(row) =
                extract_fields_from_context(&tx_type, &stock_name, year, &anchor_extra, &window)
            {
                rows.push(row);
            }

            // j now points at the next anchor (or is past the window limit).
            // Do NOT skip to j – advance by 1 so the outer loop naturally
            // reaches j on subsequent iterations (avoids skipping anchors that
            // were found inside what would have been the window).
            i += 1;
        } else {
            i += 1;
        }
    }

    // Remove exact duplicates (same time + name) that can arise when the same
    // 买入/卖出 keyword appears on two consecutive OCR lines.
    rows.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));
    rows.dedup_by(|a, b| a.traded_at == b.traded_at && a.stock_name == b.stock_name);
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

/// Returns true when the trimmed line contains a trade keyword and is
/// therefore an anchor for a new transaction record.
fn is_trade_anchor(line: &str) -> bool {
    (line.contains("买入") || line.contains("卖出"))
        && !line.starts_with("类型")
        && !line.starts_with("交易类型")
        && !line.starts_with("方向")
}

/// Try to detect a trade anchor in `line`.
///
/// Returns `(transaction_type, stock_name, anchor_extra)` where:
/// - `transaction_type` is "BUY" or "SELL".
/// - `stock_name` is the longest CJK character run found on the line (2–12 chars).
/// - `anchor_extra` is the remaining text on the anchor line after the keyword
///   and name are removed (may contain price / date / time digits).
///
/// Unlike the old `parse_trade_header`, this function detects the keyword
/// **anywhere** on the line (not only at the start) and extracts **only** the
/// CJK portion as the stock name, stopping at digits or ASCII characters.
fn detect_trade_anchor(line: &str) -> Option<(String, String, String)> {
    if !is_trade_anchor(line) {
        return None;
    }

    let tx_type = if line.contains("卖出") { "SELL" } else { "BUY" };

    // Remove the keyword so we can search for the CJK name cleanly.
    let without_keyword = line
        .replace("卖出", " ")
        .replace("买入", " ");

    let stock_name = extract_longest_cjk_run(&without_keyword)?;

    // Build the "extra" text: everything left after removing the keyword and name.
    let anchor_extra = without_keyword.replace(&stock_name as &str, " ");

    Some((tx_type.to_string(), stock_name, anchor_extra))
}

/// Kept for backward compatibility with existing unit tests.
///
/// Wraps [`detect_trade_anchor`] to return the old `(tx_type, name)` pair.
fn parse_trade_header(line: &str) -> Option<(String, String)> {
    detect_trade_anchor(line).map(|(tx, name, _)| (tx, name))
}

/// Extract the longest run of CJK characters from `s` that is between 2 and
/// 12 characters long (typical A-share stock names are 2–5 chars).
fn extract_longest_cjk_run(s: &str) -> Option<String> {
    let mut best = String::new();
    let mut current = String::new();
    for c in s.chars() {
        if is_cjk(c) {
            current.push(c);
        } else {
            if current.len() > best.len() {
                best = std::mem::take(&mut current);
            } else {
                current.clear();
            }
        }
    }
    if current.len() > best.len() {
        best = current;
    }
    if best.len() >= 2 && best.len() <= 12 {
        Some(best)
    } else {
        None
    }
}

/// Return true if `c` is a CJK Unified Ideograph (covers the vast majority of
/// Chinese characters used in A-share stock names).
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4dbf}' // Extension A
        | '\u{f900}'..='\u{faff}' // CJK Compatibility Ideographs
    )
}

/// Extract date, time, and numeric trade fields from the context lines for a
/// single transaction.
///
/// `anchor_extra` is the non-name remainder of the anchor line (may contain
/// date/time or price digits).  `window` is the subsequent non-anchor lines.
fn extract_fields_from_context(
    tx_type: &str,
    stock_name: &str,
    year: i32,
    anchor_extra: &str,
    window: &[&str],
) -> Option<ParsedTradeRow> {
    let date_re = regex::Regex::new(r"\b(\d{1,2})-(\d{2})\b").unwrap();
    let time_re = regex::Regex::new(r"\b(\d{1,2}):(\d{2})(?::\d{2})?\b").unwrap();
    let neg_re  = regex::Regex::new(r"-\d+(?:[.,]\d+)?").unwrap();
    let pct_re  = regex::Regex::new(r"\d+(?:\.\d+)?\s*%").unwrap();
    let num_re  = regex::Regex::new(r"\b(\d+(?:\.\d+)?)\b").unwrap();

    // Combine anchor extra + window into one searchable string.
    let mut parts: Vec<&str> = vec![anchor_extra];
    parts.extend_from_slice(window);
    let all_text = parts.join(" ");

    // --- Date ---
    let (month, day) = date_re
        .captures(&all_text)
        .map(|c| {
            (
                c[1].parse::<u32>().unwrap_or(1),
                c[2].parse::<u32>().unwrap_or(1),
            )
        })?;

    // --- Time ---
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

    // --- Numbers ---
    // Strip: full dates (YYYY-MM-DD), month-day (MM-DD), times (HH:MM[:SS]),
    // negative numbers (P&L like -56786.02), and percentages (4.02%).
    let full_date_re = regex::Regex::new(r"\b\d{4}-\d{2}-\d{2}\b").unwrap();
    let cleaned = full_date_re.replace_all(&all_text, " ");
    let cleaned = date_re.replace_all(&cleaned, " ");
    let cleaned = time_re.replace_all(&cleaned, " ");
    let cleaned = neg_re.replace_all(&cleaned, " ");
    let cleaned = pct_re.replace_all(&cleaned, " ");

    let numbers: Vec<f64> = num_re
        .captures_iter(&cleaned)
        .filter_map(|c| c[1].parse::<f64>().ok())
        .filter(|&n| n > 0.0)
        .collect();

    let (price, shares, total_amount, commission) = assign_fields_ordered(&numbers)?;

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

/// Maximum relative error between `price × shares` and the reported
/// `total_amount` that we accept as a consistent match.  2 % accounts for
/// rounding that occurs when the brokerage records price and total separately.
const TOTAL_MATCH_TOLERANCE: f64 = 0.02;

/// Assign (price, shares, total_amount, commission) from an ordered list of
/// positive numbers, using a three-tier strategy:
///
/// **Tier 1 – ordered search with total verification**: walk numbers in
/// document order.  For each candidate price (0 < p ≤ 10 000) find the first
/// subsequent near-integer shares (≥ 100, within ±0.5) such that a later
/// number matches `price × shares` within [`TOTAL_MATCH_TOLERANCE`].
/// Commission is the number immediately following total.
///
/// Requiring shares ≥ 100 exploits the CN market minimum lot size and rules
/// out spurious matches like "4 × 28 ≈ 112".
///
/// **Tier 2 – ordered search, shares ≥ 1**: same as tier 1 but allows odd
/// lots (< 100 shares) that arise when selling a partial position.
///
/// **Tier 3 – combinatorial fallback**: try all (i, j, k) index triples
/// regardless of order.
fn assign_fields_ordered(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if numbers.is_empty() {
        return None;
    }

    // Shared inner logic: ordered search with a minimum share count.
    let ordered_search = |min_shares: f64| -> Option<(f64, f64, f64, f64)> {
        for pi in 0..numbers.len() {
            let price = numbers[pi];
            if price <= 0.0 || price > 10_000.0 {
                continue;
            }
            for si in (pi + 1)..numbers.len() {
                let shares_raw = numbers[si];
                if shares_raw < min_shares
                    || (shares_raw - shares_raw.round()).abs() > 0.5
                {
                    continue;
                }
                let shares = shares_raw.round();
                let expected = price * shares;
                if expected <= 0.0 {
                    continue;
                }
                for ti in (si + 1)..numbers.len() {
                    let total = numbers[ti];
                    if total <= 0.0 {
                        continue;
                    }
                    let rel_err = (expected - total).abs() / total;
                    if rel_err < TOTAL_MATCH_TOLERANCE {
                        let commission = numbers.get(ti + 1).copied().unwrap_or(0.0);
                        return Some((price, shares, total, commission));
                    }
                }
            }
        }
        None
    };

    // Tier 1: CN lot size ≥ 100.
    if let Some(r) = ordered_search(100.0) {
        return Some(r);
    }

    // Tier 2: allow odd lots (≥ 1 share).
    if let Some(r) = ordered_search(1.0) {
        return Some(r);
    }

    // Tier 3: combinatorial (position-independent).
    pick_fields_combinatorial(numbers)
}

/// Combinatorial search: try all (price, shares, total) index triples
/// regardless of their document order.  Commission is any remaining number.
///
/// This is kept as a last-resort fallback for unusual layouts.
fn pick_fields_combinatorial(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if numbers.len() < 4 {
        return None;
    }
    let n = numbers.len().min(8);
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
                if rel_err < TOTAL_MATCH_TOLERANCE {
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
    None
}

/// Kept for backward compatibility with unit tests that call `pick_fields` directly.
#[cfg(test)]
fn pick_fields(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    assign_fields_ordered(numbers)
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

    // --- extract_year ---

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

    // --- parse_trade_header (backward-compat wrapper) ---

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

    // --- detect_trade_anchor ---

    /// keyword at end of line (common THS layout)
    #[test]
    fn test_detect_trade_anchor_keyword_at_end() {
        let (tx, name, extra) = detect_trade_anchor("双汇发展 卖出").unwrap();
        assert_eq!(tx, "SELL");
        assert_eq!(name, "双汇发展");
        // extra must NOT contain the CJK name
        assert!(!extra.contains("双汇发展"));
    }

    /// keyword in the middle, with numbers on same line
    #[test]
    fn test_detect_trade_anchor_with_numbers() {
        let (tx, name, extra) = detect_trade_anchor("卖出-双汇发展  28.41  -56786.02").unwrap();
        assert_eq!(tx, "SELL");
        assert_eq!(name, "双汇发展");
        // extra should contain the number but not the name
        assert!(extra.contains("28.41"));
        assert!(!extra.contains("双汇发展"));
    }

    // --- extract_longest_cjk_run ---

    #[test]
    fn test_extract_longest_cjk_run() {
        assert_eq!(
            extract_longest_cjk_run("  双汇发展  28.41"),
            Some("双汇发展".to_string())
        );
        assert_eq!(
            extract_longest_cjk_run("28.41  2000"),
            None // no CJK
        );
    }

    // --- assign_fields_ordered (replaces old pick_fields) ---

    #[test]
    fn test_pick_fields_basic() {
        // price=1505.00, shares=100 (≥100 ✓), total=150500.00, commission=5.00
        let nums = vec![1505.0f64, 100.0, 150500.0, 5.0];
        let (price, shares, total, comm) = pick_fields(&nums).unwrap();
        assert!((price - 1505.0).abs() < 0.01);
        assert!((shares - 100.0).abs() < 0.01);
        assert!((total - 150500.0).abs() < 1.0);
        assert!((comm - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_assign_fields_real_case() {
        // 双汇发展: price=28.41, shares=2000, total=56820, commission=33.98
        // Negative -56786.02 has already been removed before this call.
        let nums = vec![28.41f64, 2000.0, 56820.0, 33.98];
        let (price, shares, total, comm) = assign_fields_ordered(&nums).unwrap();
        assert!((price - 28.41).abs() < 0.01, "price={price}");
        assert!((shares - 2000.0).abs() < 0.01, "shares={shares}");
        assert!((total - 56820.0).abs() < 1.0, "total={total}");
        assert!((comm - 33.98).abs() < 0.01, "comm={comm}");
    }

    /// Extra rogue numbers before the real price (e.g. a sequence number).
    #[test]
    fn test_assign_fields_with_rogue_prefix() {
        // "1" is a rogue sequence number; "28.41 2000 56820 33.98" are the real fields.
        let nums = vec![1.0f64, 28.41, 2000.0, 56820.0, 33.98];
        let (price, shares, total, comm) = assign_fields_ordered(&nums).unwrap();
        assert!((price - 28.41).abs() < 0.01, "price={price}");
        assert!((shares - 2000.0).abs() < 0.01, "shares={shares}");
        assert!((total - 56820.0).abs() < 1.0, "total={total}");
        assert!((comm - 33.98).abs() < 0.01, "comm={comm}");
    }

    // --- parse_ths_ocr (integration) ---

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

    /// Real-world style: keyword NOT at start of line, negative P&L present.
    #[test]
    fn test_parse_ths_ocr_keyword_not_at_line_start() {
        let text = "\
2026-04
双汇发展 卖出  28.41  -56786.02
04-09  09:58   2000  56820.00  33.98
招商银行 买入  28.95  57865.44
04-22  14:26   2000  57900.00  150.00
";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 2, "expected 2 rows, got {}: {rows:?}", rows.len());

        let sell = rows.iter().find(|r| r.transaction_type == "SELL").unwrap();
        assert_eq!(sell.stock_name, "双汇发展");
        assert!((sell.price - 28.41).abs() < 0.01, "sell price={}", sell.price);
        assert!((sell.shares - 2000.0).abs() < 0.01, "sell shares={}", sell.shares);
        assert!((sell.total_amount - 56820.0).abs() < 1.0, "sell total={}", sell.total_amount);
        assert!((sell.commission - 33.98).abs() < 0.01, "sell comm={}", sell.commission);

        let buy = rows.iter().find(|r| r.transaction_type == "BUY").unwrap();
        assert_eq!(buy.stock_name, "招商银行");
        assert!((buy.price - 28.95).abs() < 0.01, "buy price={}", buy.price);
        assert!((buy.shares - 2000.0).abs() < 0.01, "buy shares={}", buy.shares);
    }

    /// All six fields on one OCR line (fully inline THS format).
    #[test]
    fn test_parse_ths_ocr_inline_format() {
        let text = "\
2026-04
卖出双汇发展 04-09 09:58 28.41 2000 56820.00 33.98
买入招商银行 04-22 14:26 28.95 2000 57900.00 150.00
";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].stock_name, "双汇发展"); // sorted: 04-09 first
        assert!((rows[0].price - 28.41).abs() < 0.01);
        assert!((rows[1].price - 28.95).abs() < 0.01);
    }
}
