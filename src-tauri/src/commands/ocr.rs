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
    /// Some Xueqiu API versions return "list" instead of "items".
    list: Option<Vec<XueqiuSearchItem>>,
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
    // Try Xueqiu first (uses the shared authenticated session).
    if let Ok(Some(code)) = lookup_via_xueqiu(&name).await {
        return Ok(Some(code));
    }
    // Fall back to Sina Suggest, which needs no cookies.
    match lookup_via_sina(&name).await {
        Ok(r) => Ok(r),
        Err(e) => Err(format!("股票代码查询失败: {e}")),
    }
}

/// Xueqiu `stock/search.json` lookup.
async fn lookup_via_xueqiu(name: &str) -> Result<Option<String>, String> {
    use std::time::Duration;

    let client = http_client::xueqiu_client();

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
        urlencoding::encode(name)
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
        .and_then(|d| d.items.or(d.list))
        .unwrap_or_default();

    for item in &items {
        let sym = match &item.symbol {
            Some(s) if !s.is_empty() => s.as_str(),
            _ => continue,
        };
        let is_cn = sym.starts_with("SH") || sym.starts_with("SZ");
        if is_cn && sym.len() == 8 {
            return Ok(Some(sym[2..].to_string()));
        }
    }

    Ok(None)
}

/// Sina Suggest API lookup — does not require any cookie or token.
///
/// Endpoint: `https://suggest3.sinajs.cn/suggest/type=11,12&key={name}`
///
/// Response: `var suggestvalue="SH600519,11,贵州茅台,...;SZ000858,...;"`
///
/// Each semicolon-delimited item has comma-separated fields:
/// `symbol_with_prefix,type,name,...`
async fn lookup_via_sina(name: &str) -> Result<Option<String>, String> {
    let url = format!(
        "https://suggest3.sinajs.cn/suggest/type=11,12&key={}",
        urlencoding::encode(name)
    );

    let client = reqwest::Client::new();
    let text = client
        .get(&url)
        .header(reqwest::header::REFERER, "https://finance.sina.com.cn")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("{e}"))?
        .text()
        .await
        .map_err(|e| format!("{e}"))?;

    // Extract the value string: var suggestvalue="...";
    let re = regex::Regex::new(r#"suggestvalue="([^"]*)""#).unwrap();
    let Some(cap) = re.captures(&text) else {
        return Ok(None);
    };
    let value = &cap[1];

    for item in value.split(';') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let sym = item.splitn(2, ',').next().unwrap_or("");
        if (sym.starts_with("SH") || sym.starts_with("SZ")) && sym.len() == 8 {
            return Ok(Some(sym[2..].to_string()));
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Image slicing — split THS screenshot into individual trade-card images
// ---------------------------------------------------------------------------

/// Minimum pixel height for a valid trade-card slice.  Slices shorter than
/// this are discarded as separator artefacts or padding-only strips.
const MIN_CARD_HEIGHT_PX: u32 = 30;

/// Minimum height (in pixels) that a separator band must span before it is
/// treated as a genuine inter-card boundary.
///
/// Compact THS list layouts use 1-3 px thin divider lines, and intra-line
/// whitespace within a single entry is typically < 8 px.  Requiring ≥ 8 px
/// prevents the algorithm from falsely splitting a 2-line entry in half.
/// Card-based layouts with wide white separators (≥ 8 px) still benefit from
/// slicing.
const MIN_SEPARATOR_BAND_PX: u32 = 8;

/// Split a THS trade-history screenshot into individual card images by
/// detecting horizontal separator bands.
///
/// THS renders each trade as a "card" in a list.  Between consecutive cards
/// there is a band of uniform light-coloured pixels (white/light-gray
/// background + optional thin divider line, typically ≥ 3 px tall).
///
/// Algorithm:
/// 1. Convert the image to grayscale (Luma8).
/// 2. For every pixel row compute the mean luminance and the pixel-value range.
/// 3. Mark rows where mean > 220 **and** range < 30 as separator candidates.
/// 4. Merge consecutive candidate rows into "separator bands".
/// 5. Cut the image at the midpoint of each band.
/// 6. Return the resulting sub-images as PNG byte vectors.
///
/// Returns `vec![data.to_vec()]` (the original image unchanged) when fewer
/// than two separator bands are found, so the caller can fall back to
/// whole-image OCR.
fn split_image_by_separators(data: &[u8]) -> Vec<Vec<u8>> {
    use image::GenericImageView as _;

    let img = match image::load_from_memory(data) {
        Ok(i) => i,
        Err(_) => return vec![data.to_vec()],
    };

    let (width, height) = img.dimensions();
    if width == 0 || height < MIN_CARD_HEIGHT_PX * 2 {
        return vec![data.to_vec()];
    }

    let gray = img.to_luma8();

    // ── 1. Label each row as a separator candidate ────────────────────────────
    let mut is_sep: Vec<bool> = vec![false; height as usize];
    for y in 0..height {
        let mut min_lum: u32 = 255;
        let mut max_lum: u32 = 0;
        let mut sum: u32 = 0;
        for x in 0..width {
            let lum = gray.get_pixel(x, y)[0] as u32;
            if lum < min_lum { min_lum = lum; }
            if lum > max_lum { max_lum = lum; }
            sum += lum;
        }
        let mean = sum / width;
        let range = max_lum - min_lum;
        is_sep[y as usize] = mean > 220 && range < 30;
    }

    // ── 2. Find separator bands (consecutive sep rows) ────────────────────────
    let mut cut_ys: Vec<u32> = Vec::new();
    let mut band_start: Option<u32> = None;
    for y in 0..height {
        match (is_sep[y as usize], band_start) {
            (true, None) => band_start = Some(y),
            (false, Some(start)) => {
                let band_height = y - start;
                // Only treat as a genuine inter-card separator when the band
                // is wide enough.  Thin 1-3 px dividers and normal intra-line
                // whitespace (< MIN_SEPARATOR_BAND_PX) are ignored so that
                // compact two-line entries are not split in half.
                if band_height >= MIN_SEPARATOR_BAND_PX {
                    cut_ys.push((start + y) / 2);
                }
                band_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = band_start {
        let band_height = height - start;
        if band_height >= MIN_SEPARATOR_BAND_PX {
            cut_ys.push((start + height) / 2);
        }
    }

    if cut_ys.is_empty() {
        return vec![data.to_vec()];
    }

    // ── 3. Build slice boundaries ─────────────────────────────────────────────
    let mut bounds: Vec<u32> = vec![0];
    bounds.extend_from_slice(&cut_ys);
    bounds.push(height);
    bounds.dedup();

    let mut slices: Vec<Vec<u8>> = Vec::new();
    for pair in bounds.windows(2) {
        let (y0, y1) = (pair[0], pair[1]);
        if y1 <= y0 || y1 - y0 < MIN_CARD_HEIGHT_PX {
            continue; // Skip slivers too thin to contain a trade card.
        }
        let sub = img.crop_imm(0, y0, width, y1 - y0);
        let mut buf: Vec<u8> = Vec::new();
        if sub
            .write_to(
                &mut std::io::Cursor::new(&mut buf),
                image::ImageFormat::Png,
            )
            .is_ok()
        {
            slices.push(buf);
        }
    }

    if slices.is_empty() {
        vec![data.to_vec()]
    } else {
        slices
    }
}

// ---------------------------------------------------------------------------
// OCR helper
// ---------------------------------------------------------------------------

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
/// # 同花顺 OCR layout (observed from tesseract chi_sim output)
///
/// Tesseract produces output like:
///
/// ```text
/// 2026-04
///
/// 贵州茅台
///
/// 卖出 2026-04-09 09:58 1459.48 100 145861.89 86.11
/// 双汇发展
///
/// 卖出 2026-04-09 13:39 28.41 2000 56786.02 33.98
/// 招商银行
///
/// 买人 2026-04-22 14:26 28.95 2000 57865.44 54.57
/// ```
///
/// Key observations from real tesseract output:
/// - **Stock name is on its own line**, separate from the direction line.
/// - **买入 is consistently OCR'd as "买人"** (入→人 misread) — must be handled.
/// - 卖出 is read correctly.
/// - The date uses full YYYY-MM-DD format on the direction line.
/// - The image "金额" (amount) is net of commission; total_amount in the DB
///   must be price × shares (gross).
///
/// Algorithm:
/// 1. Extract the year from the first `YYYY-MM` header (or YYYY-MM-DD).
/// 2. Walk lines looking for an anchor (line containing 买入/买人/卖出).
/// 3. For each anchor, find the stock name by looking backward up to 3 lines
///    (the name typically precedes the direction line).
///    If still not found, try the same anchor line (some formats embed the name).
/// 4. Collect subsequent non-anchor lines as context for field extraction.
/// 5. Compute total_amount = price × shares (do not use the OCR'd net amount).
fn parse_ths_ocr(text: &str) -> Vec<ParsedTradeRow> {
    let year = extract_year(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut rows: Vec<ParsedTradeRow> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if !is_trade_anchor(line) {
            i += 1;
            continue;
        }

        let tx_type = anchor_tx_type(line).to_string();

        // ── Find stock name ──────────────────────────────────────────────────
        // Case A: name embedded on the anchor line (e.g. "卖出双汇发展 ...").
        let (stock_name, anchor_extra) = if let Some((_, name, extra)) = detect_trade_anchor(line) {
            (name, extra)
        } else {
            // Case B: name is on a preceding line (most common THS OCR format).
            let extra = strip_trade_keywords(line);
            let mut found: Option<String> = None;
            for back in 1..=3usize {
                if i < back {
                    break;
                }
                let prev = lines[i - back].trim();
                // Don't look past another anchor.
                if is_trade_anchor(prev) {
                    break;
                }
                if let Some(name) = extract_longest_cjk_run(prev) {
                    found = Some(name);
                    break;
                }
            }
            match found {
                Some(name) => (name, extra),
                // No name found anywhere — skip this anchor.
                None => {
                    i += 1;
                    continue;
                }
            }
        };

        // ── Collect forward context ─────────────────────────────────────────
        let mut window: Vec<&str> = Vec::new();
        let mut j = i + 1;
        while j < lines.len() && window.len() < 6 {
            let l = lines[j].trim();
            if !l.is_empty() {
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

        i += 1;
    }

    // Sort chronologically; remove exact duplicates (same name + time).
    rows.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));
    rows.dedup_by(|a, b| a.traded_at == b.traded_at && a.stock_name == b.stock_name);
    rows
}

/// Return "BUY" or "SELL" for a confirmed anchor line (caller must verify
/// `is_trade_anchor` first).
fn anchor_tx_type(line: &str) -> &'static str {
    if line.contains("卖出") { "SELL" } else { "BUY" }
}

/// Remove all trade-direction keywords from `line` and return the remainder.
/// Used to build `anchor_extra` when no CJK name is on the anchor line.
fn strip_trade_keywords(line: &str) -> String {
    line.replace("卖出", " ")
        .replace("买入", " ")
        .replace("买人", " ")
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
///
/// **Important**: tesseract chi_sim consistently misreads "买入" as "买人"
/// (入 → 人) for common THS fonts, so both spellings are accepted.
fn is_trade_anchor(line: &str) -> bool {
    (line.contains("买入") || line.contains("买人") || line.contains("卖出"))
        && !line.starts_with("类型")
        && !line.starts_with("交易类型")
        && !line.starts_with("方向")
}

/// Try to detect a trade anchor in `line` where the stock name is **also on
/// the same line**.
///
/// Returns `(transaction_type, stock_name, anchor_extra)` where:
/// - `transaction_type` is "BUY" or "SELL".
/// - `stock_name` is the longest CJK character run found on the line.
/// - `anchor_extra` is the remaining text after the keyword and name are removed.
///
/// Returns `None` when no CJK stock name is found on the anchor line; callers
/// should then search preceding lines (see `parse_ths_ocr`).
fn detect_trade_anchor(line: &str) -> Option<(String, String, String)> {
    if !is_trade_anchor(line) {
        return None;
    }

    let tx_type = anchor_tx_type(line);
    let without_keyword = strip_trade_keywords(line);
    let stock_name = extract_longest_cjk_run(&without_keyword)?;
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
///
/// **total_amount** is always computed as `price × shares` and is never taken
/// from the OCR'd figure (which THS shows as the net amount after commission).
fn extract_fields_from_context(
    tx_type: &str,
    stock_name: &str,
    year: i32,
    anchor_extra: &str,
    window: &[&str],
) -> Option<ParsedTradeRow> {
    // Regex patterns compiled once per call (acceptable; `parse_ths_ocr` is
    // called infrequently and regex is fast to compile).
    let full_ymd_re = regex::Regex::new(r"\b(\d{4})-(\d{2})-(\d{2})\b").unwrap();
    let date_re     = regex::Regex::new(r"\b(\d{1,2})-(\d{2})\b").unwrap();
    let time_re     = regex::Regex::new(r"\b(\d{1,2}):(\d{2})(?::\d{2})?\b").unwrap();
    let neg_re      = regex::Regex::new(r"-\d+(?:[.,]\d+)?").unwrap();
    let pct_re      = regex::Regex::new(r"\d+(?:\.\d+)?\s*%").unwrap();
    let num_re      = regex::Regex::new(r"\b(\d+(?:\.\d+)?)\b").unwrap();

    // Combine anchor extra + window into one searchable string.
    let mut parts: Vec<&str> = vec![anchor_extra];
    parts.extend_from_slice(window);
    let all_text = parts.join(" ");

    // --- Date ---
    // Prefer the full YYYY-MM-DD pattern to avoid false matches.
    // Without this, date_re on "2026-04-09" would first match "20-26"
    // (position 0) instead of the correct "04-09".
    let (effective_year, month, day) =
        if let Some(cap) = full_ymd_re.captures(&all_text) {
            let y = cap[1].parse::<i32>().unwrap_or(year);
            let m = cap[2].parse::<u32>().unwrap_or(1);
            let d = cap[3].parse::<u32>().unwrap_or(1);
            (y, m, d)
        } else if let Some(cap) = date_re.captures(&all_text) {
            let m = cap[1].parse::<u32>().unwrap_or(1);
            let d = cap[2].parse::<u32>().unwrap_or(1);
            (year, m, d)
        } else {
            return None; // no date found → cannot form a valid trade row
        };

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
        effective_year, month, day, hour, minute
    );

    // --- Numbers ---
    // Strip full dates, short dates, times, negative numbers (P&L), percentages.
    let cleaned = full_ymd_re.replace_all(&all_text, " ");
    let cleaned = date_re.replace_all(&cleaned, " ");
    let cleaned = time_re.replace_all(&cleaned, " ");
    let cleaned = neg_re.replace_all(&cleaned, " ");
    let cleaned = pct_re.replace_all(&cleaned, " ");

    let numbers: Vec<f64> = num_re
        .captures_iter(&cleaned)
        .filter_map(|c| c[1].parse::<f64>().ok())
        .filter(|&n| n > 0.0)
        .collect();

    // assign_fields_ordered identifies price/shares/total by the constraint
    // total ≈ price × shares.  total_amount is then *overridden* with the
    // exact computed value (price × shares) because THS displays a net figure.
    let (price, shares, _ocr_total, commission) = assign_fields_ordered(&numbers)?;
    let total_amount = price * shares;

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
    if let Some(r) = pick_fields_combinatorial(numbers) {
        return Some(r);
    }

    // Tier 4: no-total fallback.
    //
    // BUY entries in the THS "对账单" layout show a *negative* net amount
    // (e.g., -59525.60) which the cleaner strips.  We are therefore left with
    // only three positive numbers: [price, shares, commission].  There is no
    // explicit total to verify against, so we compute total = price × shares
    // ourselves and verify basic sanity (total ≥ 100).
    pick_fields_no_total(numbers)
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

/// No-total fallback (Tier 4): used when the net transaction amount is
/// *negative* in the source text (BUY entries in THS "对账单" layout) and has
/// been stripped, leaving only [price, shares, commission].
///
/// Strategy: walk number pairs (pi, si) in document order.  Accept the first
/// pair where:
/// * `price` is a plausible per-share price (0 < price ≤ 10 000),
/// * `shares` is a near-integer (within ±0.5), and
/// * `price × shares` ≥ 100 (a sanity lower-bound on trade value).
///
/// Commission is the smallest remaining positive number that is less than
/// 1 % of the computed total.  Returns `None` when no valid pair is found.
fn pick_fields_no_total(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if numbers.len() < 2 {
        return None;
    }
    for pi in 0..numbers.len() {
        let price = numbers[pi];
        if price <= 0.0 || price > 10_000.0 {
            continue;
        }
        for si in 0..numbers.len() {
            if si == pi {
                continue;
            }
            let shares_raw = numbers[si];
            if shares_raw < 1.0 || (shares_raw - shares_raw.round()).abs() > 0.5 {
                continue;
            }
            let shares = shares_raw.round();
            let total = price * shares;
            if total < 100.0 {
                continue;
            }
            // Commission is any remaining number smaller than 1 % of total.
            let commission_cap = total * 0.01;
            let commission = numbers
                .iter()
                .enumerate()
                .filter(|(idx, _)| *idx != pi && *idx != si)
                .map(|(_, &v)| v)
                .find(|&v| v > 0.0 && v < commission_cap)
                .unwrap_or(0.0);
            return Some((price, shares, total, commission));
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

/// Decode a base64-encoded image, pre-slice it by separator bands, run
/// Tesseract OCR on each slice, and return the merged parsed trade rows.
///
/// Slicing each trade card into its own image dramatically improves OCR
/// accuracy because Tesseract no longer has to deal with cross-card layout
/// ambiguity.
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

    // Split the image into per-card slices (falls back to the whole image when
    // no separator lines are detected).
    let slices = split_image_by_separators(&bytes);

    let mut all_rows: Vec<ParsedTradeRow> = if slices.len() <= 1 {
        // No separators found — OCR the whole image as before.
        let text = ocr_image(&bytes)?;
        parse_ths_ocr(&text)
    } else {
        let mut combined: Vec<ParsedTradeRow> = Vec::new();
        for slice in &slices {
            if let Ok(text) = ocr_image(slice) {
                combined.extend(parse_ths_ocr(&text));
            }
        }
        combined
    };

    // Deduplicate and sort chronologically.
    all_rows.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));
    all_rows.dedup_by(|a, b| a.traded_at == b.traded_at && a.stock_name == b.stock_name);
    Ok(all_rows)
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

    // --- is_trade_anchor handles 买人 (OCR misread of 买入) ---

    #[test]
    fn test_is_trade_anchor_mai_ren() {
        assert!(is_trade_anchor("买人 2026-04-22 14:26 28.95 2000 57865.44 54.57"));
        // Should be classified BUY, not SELL
        assert_eq!(anchor_tx_type("买人 28.95 2000"), "BUY");
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

    /// 买人 (tesseract misread) on anchor line — no CJK name on same line
    #[test]
    fn test_detect_trade_anchor_mai_ren_no_name() {
        // Direction line has no stock name; detect_trade_anchor returns None.
        // parse_ths_ocr should then look backward.
        assert!(detect_trade_anchor("买人 2026-04-22 14:26 28.95 2000 57865.44 54.57").is_none());
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

    /// Inline format: name + keyword on same line.
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
        // total_amount must be computed (price × shares), not taken from OCR.
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

    /// Total_amount is always price × shares, not the OCR'd net amount.
    #[test]
    fn test_total_amount_computed_from_price_times_shares() {
        // THS shows net amount 57865.44 (after commission 54.57).
        // DB must store gross: 28.95 × 2000 = 57900.
        let text = "2026-04\n买入-招商银行\n04-22 14:26 28.95 2000 57865.44 54.57\n";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 1);
        let expected = 28.95 * 2000.0;
        assert!(
            (rows[0].total_amount - expected).abs() < 1.0,
            "total={}, expected={}",
            rows[0].total_amount,
            expected
        );
    }

    /// Full YYYY-MM-DD date on the anchor line — must not produce month=20 day=26.
    #[test]
    fn test_parse_ths_ocr_full_date_format() {
        let text = "买入-招商银行 2026-04-22 14:26 28.95 2000 57900.00 150.00\n";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 1, "expected 1 row, got {}", rows.len());
        assert_eq!(rows[0].traded_at, "2026-04-22T14:26:00");
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

    // ── Real THS OCR format tests ────────────────────────────────────────────
    // Observed from running `tesseract chi_sim` on a synthetic THS-style image:
    //   - Stock name appears on its OWN line.
    //   - Direction is on the NEXT line (no stock name).
    //   - 买入 is consistently misread as "买人" by tesseract.
    //   - Full YYYY-MM-DD format is used for dates.

    /// Name-before-direction format (the most common real-world THS OCR output).
    #[test]
    fn test_parse_ths_ocr_name_before_direction() {
        let text = "\
2026-04
双汇发展
卖出 2026-04-09 09:58 28.41 2000 56786.02 33.98
招商银行
买人 2026-04-22 14:26 28.95 2000 57865.44 54.57
";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 2, "expected 2 rows, got {}: {rows:?}", rows.len());

        let sell = rows.iter().find(|r| r.transaction_type == "SELL").unwrap();
        assert_eq!(sell.stock_name, "双汇发展");
        assert!((sell.price - 28.41).abs() < 0.01, "sell price={}", sell.price);
        assert!((sell.shares - 2000.0).abs() < 0.01, "sell shares={}", sell.shares);
        // total_amount must be price × shares, not the OCR'd net amount.
        assert!(
            (sell.total_amount - 28.41 * 2000.0).abs() < 1.0,
            "sell total={} (expected {})",
            sell.total_amount,
            28.41 * 2000.0
        );
        assert!((sell.commission - 33.98).abs() < 0.01, "sell comm={}", sell.commission);
        assert_eq!(sell.traded_at, "2026-04-09T09:58:00");

        let buy = rows.iter().find(|r| r.transaction_type == "BUY").unwrap();
        assert_eq!(buy.stock_name, "招商银行");
        assert!((buy.price - 28.95).abs() < 0.01, "buy price={}", buy.price);
        assert!((buy.shares - 2000.0).abs() < 0.01, "buy shares={}", buy.shares);
        assert_eq!(buy.traded_at, "2026-04-22T14:26:00");
    }

    /// 买人 (tesseract misread of 买入) must be detected as BUY.
    #[test]
    fn test_parse_ths_ocr_mai_ren_ocr_misread() {
        let text = "\
2026-04
招商银行
买人 2026-04-22 14:26 28.95 2000 57865.44 54.57
";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 1, "expected 1 BUY row, got {}", rows.len());
        assert_eq!(rows[0].transaction_type, "BUY");
        assert_eq!(rows[0].stock_name, "招商银行");
        assert!((rows[0].price - 28.95).abs() < 0.01);
    }

    /// Six records: name-before-direction format with 买人 misreads (real OCR output).
    /// This is the exact format tesseract chi_sim produces from a THS screenshot.
    #[test]
    fn test_parse_ths_ocr_six_records_real_ocr_format() {
        // This text was produced by running tesseract chi_sim on a synthetic
        // THS-style image (see ocr_test_image.rs / scripts/gen_ths_img.py).
        let text = "\
2026-04

贵州茅台

卖出 2026-04-09 09:58 1459.48 100 145861.89 86.11
双汇发展

卖出 2026-04-09 13:39 28.41 2000 56786.02 33.98
招商银行

买人 2026-04-22 14:26 28.95 2000 57865.44 54.57
平安银行

买人 2026-04-15 10:30 12.50 1000 12487.50 12.50
工商银行

卖出 2026-04-20 14:00 5.80 2000 11588.00 12.00
中国石油

买人 2026-04-25 09:45 7.20 3000 21578.40 21.60
";
        let rows = parse_ths_ocr(text);
        assert_eq!(
            rows.len(),
            6,
            "expected 6 rows, got {}: {:?}",
            rows.len(),
            rows.iter().map(|r| format!("{}/{}", r.stock_name, r.transaction_type)).collect::<Vec<_>>()
        );

        // Verify a sample of expected values.
        let maotai = rows.iter().find(|r| r.stock_name.contains("贵州茅台")).unwrap();
        assert_eq!(maotai.transaction_type, "SELL");
        assert!((maotai.price - 1459.48).abs() < 0.01, "maotai price={}", maotai.price);
        assert!((maotai.shares - 100.0).abs() < 0.01);
        assert!(
            (maotai.total_amount - 1459.48 * 100.0).abs() < 1.0,
            "total={} expected={}",
            maotai.total_amount, 1459.48 * 100.0
        );

        let zhaoshang = rows.iter().find(|r| r.stock_name.contains("招商银行")).unwrap();
        assert_eq!(zhaoshang.transaction_type, "BUY");
        assert!((zhaoshang.price - 28.95).abs() < 0.01);
        assert!((zhaoshang.shares - 2000.0).abs() < 0.01);
    }

    // ── split_image_by_separators ────────────────────────────────────────────

    /// Build a synthetic PNG with N cards separated by uniform light-gray bands.
    /// Returns the raw PNG bytes.
    #[cfg(test)]
    fn make_test_image_with_separators(n_cards: u32) -> Vec<u8> {
        use image::{ImageBuffer, Rgb};
        let card_h: u32 = 80;
        let sep_h: u32 = 10; // 10 px ≥ MIN_SEPARATOR_BAND_PX (8) → emitted as a cut
        let width: u32 = 400;
        // Total height: n cards + (n-1) separators
        let total_h = n_cards * card_h + (n_cards.saturating_sub(1)) * sep_h;
        let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(width, total_h);

        let card_color = Rgb([50u8, 50, 50]); // dark content
        let sep_color = Rgb([235u8, 235, 235]); // light separator

        for y in 0..total_h {
            // Determine which "stripe" this row belongs to.
            let stripe_h = card_h + sep_h;
            let local_y = y % stripe_h;
            let color = if local_y < card_h { card_color } else { sep_color };
            for x in 0..width {
                img.put_pixel(x, y, color);
            }
        }

        let mut buf: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("encode test image");
        buf
    }

    #[test]
    fn test_split_one_card_returns_whole_image() {
        // Single card: no separator bands → function returns original bytes.
        let bytes = make_test_image_with_separators(1);
        let slices = split_image_by_separators(&bytes);
        assert_eq!(slices.len(), 1, "expected 1 slice for single card");
    }

    #[test]
    fn test_split_two_cards_produces_two_slices() {
        let bytes = make_test_image_with_separators(2);
        let slices = split_image_by_separators(&bytes);
        assert_eq!(slices.len(), 2, "expected 2 slices for 2-card image, got {}", slices.len());
    }

    #[test]
    fn test_split_six_cards_produces_six_slices() {
        let bytes = make_test_image_with_separators(6);
        let slices = split_image_by_separators(&bytes);
        assert_eq!(slices.len(), 6, "expected 6 slices for 6-card image, got {}", slices.len());
    }

    #[test]
    fn test_split_invalid_bytes_returns_original() {
        let bad = b"not an image".to_vec();
        let slices = split_image_by_separators(&bad);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0], bad);
    }

    /// Thin 1-4 px separator bands are ignored — the compact THS list layout
    /// should NOT be sliced into per-entry fragments (which would separate
    /// line 1 from line 2 of the same entry).
    #[test]
    fn test_split_thin_separators_not_cut() {
        use image::{ImageBuffer, Rgb};
        // Build image with 4px thin light-gray separator bands between dark cards.
        let card_h: u32 = 80;
        let sep_h: u32 = 4; // below MIN_SEPARATOR_BAND_PX=8 → not treated as cut
        let n_cards: u32 = 3;
        let width: u32 = 400;
        let total_h = n_cards * card_h + (n_cards - 1) * sep_h;
        let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(width, total_h);
        let card_color = Rgb([50u8, 50, 50]);
        let sep_color = Rgb([235u8, 235, 235]);
        for y in 0..total_h {
            let stripe = card_h + sep_h;
            let col = if y % stripe < card_h { card_color } else { sep_color };
            for x in 0..width { img.put_pixel(x, y, col); }
        }
        let mut buf: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("encode");
        let slices = split_image_by_separators(&buf);
        // Thin separators (< MIN_SEPARATOR_BAND_PX) should be ignored →
        // no cuts → falls back to returning the original image as a single slice.
        assert_eq!(slices.len(), 1, "thin separators must not trigger slicing, got {} slices", slices.len());
    }

    // ── pick_fields_no_total (Tier 4) ────────────────────────────────────────

    /// BUY entries in THS 对账单 have a negative net amount which gets stripped.
    /// Only [price, shares, commission] remain.  Tier 4 must handle this.
    #[test]
    fn test_pick_fields_no_total_buy_entry() {
        // 招商银行: price=39.680, shares=1500, commission=5.60
        let nums = vec![39.680f64, 1500.0, 5.60];
        let (price, shares, total, comm) = pick_fields_no_total(&nums).unwrap();
        assert!((price - 39.680).abs() < 0.01, "price={price}");
        assert!((shares - 1500.0).abs() < 0.01, "shares={shares}");
        assert!((total - 39.680 * 1500.0).abs() < 1.0, "total={total}");
        assert!((comm - 5.60).abs() < 0.01, "comm={comm}");
    }

    #[test]
    fn test_pick_fields_no_total_with_small_price() {
        // 双汇发展: price=28.95, shares=2000, commission=34.57
        let nums = vec![28.95f64, 2000.0, 34.57];
        let (price, shares, total, comm) = pick_fields_no_total(&nums).unwrap();
        assert!((price - 28.95).abs() < 0.01);
        assert!((shares - 2000.0).abs() < 0.01);
        assert!((total - 57900.0).abs() < 1.0);
        assert!((comm - 34.57).abs() < 0.01);
    }

    #[test]
    fn test_pick_fields_no_total_returns_none_for_single_number() {
        assert!(pick_fields_no_total(&[100.0]).is_none());
        assert!(pick_fields_no_total(&[]).is_none());
    }

    /// End-to-end: THS 对账单 format with BUY entries having negative net amounts.
    /// This is the actual format from the user's screenshot.
    #[test]
    fn test_parse_ths_ocr_duizhangsingle_buy_negative_amount() {
        // 买入-招商银行  price  -net_amount
        // MM-DD HH:MM   shares  commission
        let text = "\
2026-04
买入-招商银行    39.680  -59525.60
04-22 14:26              1500       5.60
";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 1, "expected 1 BUY row, got {}: {rows:?}", rows.len());
        let r = &rows[0];
        assert_eq!(r.transaction_type, "BUY");
        assert_eq!(r.stock_name, "招商银行");
        assert!((r.price - 39.680).abs() < 0.01, "price={}", r.price);
        assert!((r.shares - 1500.0).abs() < 0.01, "shares={}", r.shares);
        assert!((r.total_amount - 39.680 * 1500.0).abs() < 1.0, "total={}", r.total_amount);
        assert!((r.commission - 5.60).abs() < 0.01, "comm={}", r.commission);
    }

    /// End-to-end: full THS 对账单 page with 3 BUYs + 3 SELLs as shown in
    /// the user's real screenshot.
    #[test]
    fn test_parse_ths_ocr_duizhangdan_six_mixed_entries() {
        let text = "\
2026-04
买入-招商银行    39.680  -59525.60
04-22 14:26              1500       5.60
卖出-双汇发展   28.950    57865.43
04-22 14:26              2000      34.57
买入-招商银行   38.970   -58460.58
04-13 09:59              1500       5.58
卖出-双汇发展   28.410    56786.02
04-13 09:58              2000      33.98
买入-招商银行   39.280  -145349.09
04-09 13:59              3700      13.09
卖出-贵州茅台  1459.480  145861.89
04-09 13:39              100       86.11
";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 6, "expected 6 rows, got {}: {rows:?}", rows.len());

        let buys: Vec<_> = rows.iter().filter(|r| r.transaction_type == "BUY").collect();
        let sells: Vec<_> = rows.iter().filter(|r| r.transaction_type == "SELL").collect();
        assert_eq!(buys.len(), 3, "expected 3 BUY rows");
        assert_eq!(sells.len(), 3, "expected 3 SELL rows");

        // Check the 贵州茅台 sell
        let maotai = sells.iter().find(|r| r.stock_name.contains("贵州茅台")).unwrap();
        assert!((maotai.price - 1459.480).abs() < 0.01);
        assert!((maotai.shares - 100.0).abs() < 0.01);
        assert!((maotai.total_amount - 1459.480 * 100.0).abs() < 1.0);

        // Check a 招商银行 buy — both 1500-share entries should be present
        // with their actual prices.  After chronological sort the 04-13 entry
        // (38.970) precedes the 04-22 entry (39.680), so use explicit find.
        let zhaoshang_buy_0422 = buys.iter()
            .find(|r| r.stock_name.contains("招商银行") && (r.shares - 1500.0).abs() < 1.0
                && r.traded_at.contains("04-22"))
            .expect("04-22 招商银行 1500-share buy not found");
        assert!((zhaoshang_buy_0422.price - 39.680).abs() < 0.01,
            "price={}", zhaoshang_buy_0422.price);
        assert!((zhaoshang_buy_0422.commission - 5.60).abs() < 0.01,
            "commission={}", zhaoshang_buy_0422.commission);
    }

    /// End-to-end test using the EXACT Tesseract OCR text produced from our
    /// synthetic THS 对账单 image (verified by running tesseract chi_sim on the
    /// image and capturing stdout).  This is the closest we can get to a real
    /// integration test without a real device.
    #[test]
    fn test_parse_actual_tesseract_output() {
        // This is the verbatim output from: tesseract ths_synthetic.png out -l chi_sim --psm 6
        let text = "\
本月操作                                                 价格/数量             金额/税费 四
V 2026-04                   +270,742.49 +1.68%
买入-招商银行                                           39.680            -59525.60
@@ 04-22 14:26                                               1500                5.60
卖出-双汇发展                                           28.950            57865.43
@@ 04-22 14:26                                               2000                34.57
买入-招商银行                                           38.970            -58460.58
@@ 04-13 09:59                            1500          5.58
卖出-双汇发展                                           28.410            56786.02
@@ 04-13 09:58                                                  2000                  33.98
买入-招商银行                                           39.280            -145349.09
@@ 04-09 13:59                                               3700                 13.09
卖出-贵州茅台                                            1459.480        145861.89
@@ 04-09 13:39                                               100                  86.11
V 2026-03                -151,661.89 -1.00%
V 2026-02                 +74,518.99 +0.47%
";
        let rows = parse_ths_ocr(text);
        assert_eq!(rows.len(), 6, "expected 6 rows from real tesseract output, got {}: {rows:?}", rows.len());

        let buys:  Vec<_> = rows.iter().filter(|r| r.transaction_type == "BUY").collect();
        let sells: Vec<_> = rows.iter().filter(|r| r.transaction_type == "SELL").collect();
        assert_eq!(buys.len(),  3, "expected 3 BUY rows, got {buys:?}");
        assert_eq!(sells.len(), 3, "expected 3 SELL rows, got {sells:?}");

        // Spot-check the 贵州茅台 sell
        let maotai = sells.iter().find(|r| r.stock_name.contains("贵州茅台"))
            .expect("贵州茅台 SELL not found");
        assert!((maotai.price  - 1459.480).abs() < 0.01, "maotai price={}", maotai.price);
        assert!((maotai.shares - 100.0).abs()    < 0.01, "maotai shares={}", maotai.shares);

        // Spot-check the 04-22 招商银行 buy
        let zhaoshang = buys.iter()
            .find(|r| r.stock_name.contains("招商银行") && r.traded_at.contains("04-22"))
            .expect("招商银行 BUY 04-22 not found");
        assert!((zhaoshang.price  - 39.680).abs() < 0.01, "zhaoshang price={}", zhaoshang.price);
        assert!((zhaoshang.shares - 1500.0).abs() < 0.01, "zhaoshang shares={}", zhaoshang.shares);
        assert!((zhaoshang.commission - 5.60).abs() < 0.01, "zhaoshang commission={}", zhaoshang.commission);
    }
}
