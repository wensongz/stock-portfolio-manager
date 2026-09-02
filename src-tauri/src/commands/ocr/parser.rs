use super::*;

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
pub(super) fn parse_ths_ocr(text: &str) -> Vec<ParsedTradeRow> {
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

    // ── Dateline fallback ─────────────────────────────────────────────────────
    // When Tesseract cannot read CJK trade-direction keywords (买入/卖出) — which
    // happens on small-font phone screenshots — the anchor-based pass above
    // returns zero rows.  In that case, fall back to detecting the secondary
    // "★ / 日 MM-DD HH:MM  shares  commission" line and working backward to get
    // the price and amount from the preceding line.
    //
    // Layout (2-line per trade, as seen on iPhone THS app):
    //   Line N  : [garbled-name]  price  ±amount          ← price row
    //   Line N+1: (日|★|@) MM-DD HH[.:]MM  shares  comm   ← date row
    if rows.is_empty() {
        if let Some(fallback) = parse_ths_ocr_dateline_fallback(text, year, &lines) {
            return fallback;
        }
    }

    rows
}

/// Return "BUY" or "SELL" for a confirmed anchor line (caller must verify
/// `is_trade_anchor` first).
pub(super) fn anchor_tx_type(line: &str) -> &'static str {
    if line.contains("卖出") {
        "SELL"
    } else {
        "BUY"
    }
}

/// Remove all trade-direction keywords from `line` and return the remainder.
/// Used to build `anchor_extra` when no CJK name is on the anchor line.
pub(super) fn strip_trade_keywords(line: &str) -> String {
    line.replace("卖出", " ")
        .replace("买入", " ")
        .replace("买人", " ")
}

/// Fallback parser for the 2-line-per-trade THS layout when the primary
/// anchor-based pass returns zero rows (because Tesseract could not read the
/// CJK direction keywords "买入"/"卖出" and they appeared garbled or blank).
///
/// # Layout observed in 2× phone-screenshot OCR output
///
/// ```text
/// 日2026-04    +270.742.49
///              39.680    -59525.60    ← price row (name garbled/blank)
/// 日04-22 14.26    1500    5.60       ← date row  (★ misread as 日)
///              28.950    57865.43
/// 日04-22 14.26    2000    3457
/// ...
/// ```
///
/// The date row is identified by the pattern `(日|@|★)? MM[.:-]DD HH[.:]MM`
/// (possibly run together as `MMDDHHMI`).  The price row immediately precedes it.
///
/// BUY vs SELL is inferred from the sign of the amount on the price row:
/// negative → BUY (money left account), positive → SELL.
///
/// Stock name: look backward from the price row for the first CJK run (≥ 2
/// chars) not in a group-header line.  If none found, use "未知" ("unknown")
/// so the row is still returned — the user can correct it in the review table.
pub(super) fn parse_ths_ocr_dateline_fallback(
    _text: &str,
    year: i32,
    lines: &[&str],
) -> Option<Vec<ParsedTradeRow>> {
    // Regex for a "date row": optional non-digit prefix, then MM-DD or MM.DD,
    // then optional space, then HH:MM or HH.MM or HHMM.
    let dateline_re =
        regex::Regex::new(r"^[^\d]*(\d{1,2})[.\-](\d{2})\s*[.\-:]?(\d{2})[.\-:]?(\d{2})").unwrap();
    // For extracting numbers that follow AFTER the date/time portion.
    let pos_num_re = regex::Regex::new(r"\b(\d+(?:\.\d+)?)\b").unwrap();
    let all_num_re = regex::Regex::new(r"-?\d+(?:\.\d+)?").unwrap();
    // For skipping group-header lines containing a 4-digit year (e.g. "∧ 2026-04").
    let year_re = regex::Regex::new(r"\b\d{4}\b").unwrap();

    let mut rows: Vec<ParsedTradeRow> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let cap = match dateline_re.captures(trimmed) {
            Some(c) => c,
            None => continue,
        };
        let month: u32 = cap[1].parse().unwrap_or(0);
        let day: u32 = cap[2].parse().unwrap_or(0);
        let hour: u32 = cap[3].parse().unwrap_or(9);
        let minute: u32 = cap[4].parse().unwrap_or(30);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            continue;
        }
        let traded_at = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:00",
            year, month, day, hour, minute
        );

        // ── Extract shares and commission from the date row itself ────────────
        // Use the END position of the date/time regex match to skip over date
        // digits and only parse numbers that come AFTER the date portion.
        // This avoids mis-counting tokens when "HH.MM" is ONE regex match vs
        // when "HH:MM" splits into two.
        let date_match_end = cap.get(0).unwrap().end();
        let after_date = &trimmed[date_match_end..];
        let pos_after_date: Vec<f64> = pos_num_re
            .captures_iter(after_date)
            .filter_map(|c| c[1].parse::<f64>().ok())
            .filter(|&n| n > 0.0)
            .collect();
        // shares is the first near-integer ≥ 1; commission is the next.
        let shares_f = pos_after_date
            .iter()
            .copied()
            .find(|&n| n >= 1.0 && (n - n.round()).abs() < 0.5);
        let commission = pos_after_date
            .iter()
            .copied()
            .find(|&n| Some(n) != shares_f)
            .unwrap_or(0.0);

        // ── Find price row = first non-empty, non-dateline line above ─────────
        let price_line = (0..i)
            .rev()
            .map(|k| lines[k].trim())
            .find(|l| !l.is_empty() && !dateline_re.is_match(l));
        let price_line = match price_line {
            Some(l) => l,
            None => continue,
        };
        let price_idx = lines[..i]
            .iter()
            .rposition(|l| l.trim() == price_line)
            .unwrap_or(i.saturating_sub(1));

        // All numbers on the price row.
        let all_nums: Vec<f64> = all_num_re
            .find_iter(price_line)
            .filter_map(|m| m.as_str().parse::<f64>().ok())
            .collect();
        if all_nums.is_empty() {
            continue;
        }
        let pos_nums: Vec<f64> = pos_num_re
            .captures_iter(price_line)
            .filter_map(|c| c[1].parse::<f64>().ok())
            .collect();

        // BUY/SELL inferred from sign of amount.
        let amount_opt = all_nums.iter().copied().find(|&n| n < 0.0);
        let (tx_type, price_opt) = if let Some(_neg) = amount_opt {
            // negative amount → BUY; price is the smallest positive < 10 000
            (
                "BUY",
                pos_nums.iter().copied().find(|&n| n > 0.0 && n < 10_000.0),
            )
        } else if pos_nums.len() >= 2 {
            // All positive: infer from scale.
            // Find the smallest positive number that could be a per-share price
            // (< 10 000), then check if any OTHER positive is > price × 1.5
            // (i.e. it's the total amount, not another price).  If so → SELL.
            let price_cand = pos_nums
                .iter()
                .copied()
                .filter(|&n| n > 0.0 && n < 10_000.0)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let amount_cand =
                price_cand.and_then(|p| pos_nums.iter().copied().find(|&n| n > p * 1.5));
            if price_cand.is_some() && amount_cand.is_some() {
                ("SELL", price_cand)
            } else {
                ("BUY", pos_nums.first().copied())
            }
        } else {
            ("BUY", pos_nums.first().copied())
        };

        let price = match price_opt {
            Some(p) if p > 0.0 => p,
            _ => continue,
        };
        let shares = match shares_f {
            Some(s) if s >= 1.0 => s.round(),
            _ => continue,
        };

        // ── Stock name: search backward for CJK run (≥ 2 Unicode chars) ───────
        // Check char count (not byte length) so single CJK chars like "日"
        // (3 bytes but 1 char) are not accepted as stock names.
        let stock_name = std::iter::once(price_line)
            .chain((0..price_idx).rev().map(|k| lines[k].trim()))
            .take(4)
            .find_map(|l| {
                // Skip group-header lines containing a year number (e.g. "∧ 2026-04").
                if year_re.is_match(l) {
                    return None;
                }
                extract_longest_cjk_run(l).filter(|name| name.chars().count() >= 2)
            })
            .unwrap_or_else(|| "未知".to_string());

        let total_amount = price * shares;
        rows.push(ParsedTradeRow {
            transaction_type: tx_type.to_string(),
            stock_name,
            traded_at,
            price,
            shares,
            total_amount,
            commission,
        });
    }

    if rows.is_empty() {
        return None;
    }

    // Sort chronologically; deduplicate.
    // Use (traded_at, price, shares) as the key so that two trades at the same
    // timestamp but with different prices/shares are NOT removed (this happens
    // when stock names are both "未知" due to garbled OCR).
    rows.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));
    rows.dedup_by(|a, b| {
        a.traded_at == b.traded_at
            && a.stock_name == b.stock_name
            && (a.price - b.price).abs() < 0.001
            && (a.shares - b.shares).abs() < 0.001
    });
    Some(rows)
}

/// Return the 4-digit year found in the first `YYYY-MM` or `YYYY.MM` header
/// in `text`.  Falls back to the current UTC year if none is found.
///
/// NOTE: The `\b` word-boundary assertion does not fire between CJK characters
/// (like "日") and ASCII digits in Rust's regex engine, so we use a raw byte
/// "not preceded by ASCII digit" guard instead.
pub(super) fn extract_year(text: &str) -> i32 {
    // Match YYYY-MM at the start of a line (possibly preceded by CJK chars).
    let re = regex::Regex::new(r"(?m)(\d{4})[-.](\d{2})").unwrap();
    for cap in re.captures_iter(text) {
        let start = cap.get(1).unwrap().start();
        // Reject if preceded by a digit (avoids matching inside e.g. "12345-06").
        if start > 0 && text.as_bytes()[start - 1].is_ascii_digit() {
            continue;
        }
        let y: i32 = match cap[1].parse() {
            Ok(y) => y,
            Err(_) => continue,
        };
        let m: u32 = match cap[2].parse() {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Accept a year only if the month component is plausible (1–12) and the
        // year is within a reasonable range of the current year (±10 years).
        let current_year = chrono::Utc::now().year();
        if y >= current_year - 10 && y <= current_year + 2 && (1..=12).contains(&m) {
            return y;
        }
    }
    use chrono::Datelike as _;
    chrono::Utc::now().year()
}

/// Returns true when the trimmed line contains a trade keyword and is
/// therefore an anchor for a new transaction record.
///
/// **Important**: tesseract chi_sim consistently misreads "买入" as "买人"
/// (入 → 人) for common THS fonts, so both spellings are accepted.
pub(super) fn is_trade_anchor(line: &str) -> bool {
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
pub(super) fn detect_trade_anchor(line: &str) -> Option<(String, String, String)> {
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
#[cfg(test)]
pub(super) fn parse_trade_header(line: &str) -> Option<(String, String)> {
    detect_trade_anchor(line).map(|(tx, name, _)| (tx, name))
}

/// Extract the longest run of CJK characters from `s` that is between 2 and
/// 12 characters long (typical A-share stock names are 2–5 chars).
pub(super) fn extract_longest_cjk_run(s: &str) -> Option<String> {
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
pub(super) fn is_cjk(c: char) -> bool {
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
pub(super) fn extract_fields_from_context(
    tx_type: &str,
    stock_name: &str,
    year: i32,
    anchor_extra: &str,
    window: &[&str],
) -> Option<ParsedTradeRow> {
    // Regex patterns compiled once per call (acceptable; `parse_ths_ocr` is
    // called infrequently and regex is fast to compile).
    let full_ymd_re = regex::Regex::new(r"\b(\d{4})-(\d{2})-(\d{2})\b").unwrap();
    // Accept both hyphen and period as the date separator.  Tesseract on small
    // phone screenshots often reads "04-22" as "04.22".
    //
    // NOTE: no leading \b.  The Rust regex crate treats Unicode characters
    // (including CJK, e.g. "全") as \w, so there is NO word boundary between
    // "全" and a following ASCII digit.  Instead we apply a manual "not preceded
    // by an ASCII digit" guard inside the filter_map / closure below.
    let date_re = regex::Regex::new(r"(\d{1,2})[.-](\d{2})").unwrap();
    // Same but also optionally consumes a run-together HHMM that immediately
    // follows the date (e.g. "04.221426" ← merged "04-22 14:26").
    // The captured HHMM is discarded; we want to strip the spurious digits so
    // they don't pollute number extraction.
    let date_clean_re =
        regex::Regex::new(r"(\d{1,2})[.-](\d{2})(?:\s*([01]\d|2[0-3])([0-5]\d)\b)?").unwrap();
    let time_re = regex::Regex::new(r"\b(\d{1,2}):(\d{2})(?::\d{2})?\b").unwrap();
    let neg_re = regex::Regex::new(r"-\d+(?:[.,]\d+)?").unwrap();
    let pct_re = regex::Regex::new(r"\d+(?:\.\d+)?\s*%").unwrap();
    let num_re = regex::Regex::new(r"\b(\d+(?:\.\d+)?)\b").unwrap();

    // Combine anchor extra + window into one searchable string.
    let mut parts: Vec<&str> = vec![anchor_extra];
    parts.extend_from_slice(window);
    let all_text = parts.join(" ");

    // Helper: check that the byte immediately before position `pos` in `s` is
    // NOT an ASCII digit.  This is the manual lookbehind that replaces the
    // leading \b (which doesn't fire between CJK and ASCII digits in Rust regex).
    let not_preceded_by_digit =
        |s: &str, pos: usize| -> bool { pos == 0 || !s.as_bytes()[pos - 1].is_ascii_digit() };

    // --- Date (3-tier cascade) ---
    //
    // Tier A – full YYYY-MM-DD (e.g. section-header lines).
    // Tier B – MM-DD or MM.DD with a separator character.
    //          No leading \b: Rust regex treats CJK chars (like "全") as \w,
    //          so \b would not fire between "全" and the following digit.
    //          A manual "not preceded by ASCII digit" guard replaces it.
    // Tier C – 8-digit MMDDHHMI blob (e.g. "04091353") produced when OCR
    //          merges date *and* time with all separators lost.
    let (effective_year, month, day) = if let Some(cap) = full_ymd_re.captures(&all_text) {
        let y = cap[1].parse::<i32>().unwrap_or(year);
        let m = cap[2].parse::<u32>().unwrap_or(1);
        let d = cap[3].parse::<u32>().unwrap_or(1);
        (y, m, d)
    } else if let Some((m, d)) = date_re
        .captures_iter(&all_text)
        .filter_map(|cap| {
            let start = cap.get(0)?.start();
            if !not_preceded_by_digit(&all_text, start) {
                return None;
            }
            let m = cap[1].parse::<u32>().ok()?;
            let d = cap[2].parse::<u32>().ok()?;
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                Some((m, d))
            } else {
                None
            }
        })
        .next()
    {
        (year, m, d)
    } else if let Some((m, d)) = {
        // Tier C: isolated 8-digit blob – first 4 digits are MMDD.
        // Bind eight_re to a local so its lifetime doesn't escape the block.
        let eight_re = regex::Regex::new(r"\d{8}").unwrap();
        let result = eight_re.find_iter(&all_text).find_map(|m_match| {
            let start = m_match.start();
            let end = m_match.end();
            if !not_preceded_by_digit(&all_text, start) {
                return None;
            }
            if end < all_text.len() && all_text.as_bytes()[end].is_ascii_digit() {
                return None;
            }
            let s = m_match.as_str();
            let month: u32 = s[0..2].parse().ok()?;
            let day: u32 = s[2..4].parse().ok()?;
            if (1..=12).contains(&month) && (1..=31).contains(&day) {
                Some((month, day))
            } else {
                None
            }
        });
        result
    } {
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
    // Strip full dates, short dates (+ merged HHMM), times, negatives, pcts.
    // The date_clean_re closure applies the same manual "not preceded by digit"
    // guard and month/day range check used in the date-capture step above.
    let cleaned = full_ymd_re.replace_all(&all_text, " ").into_owned();
    let cleaned = {
        let src: &str = &cleaned;
        date_clean_re.replace_all(src, |cap: &regex::Captures<'_>| {
            let start = cap.get(0).unwrap().start();
            // Skip if preceded by an ASCII digit (avoids removing part of a
            // price like "39.68" when the full number is "39.680").
            if !not_preceded_by_digit(src, start) {
                return cap[0].to_string();
            }
            let m = cap[1].parse::<u32>().unwrap_or(99);
            let d = cap[2].parse::<u32>().unwrap_or(99);
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                " ".to_string()
            } else {
                cap[0].to_string() // not a valid date → keep
            }
        })
    };
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
pub(super) fn assign_fields_ordered(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
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
                if shares_raw < min_shares || (shares_raw - shares_raw.round()).abs() > 0.5 {
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
                        let commission_raw = numbers.get(ti + 1).copied().unwrap_or(0.0);
                        // Sanity-check: A-share trading costs are at most ~0.4% of trade
                        // value for large trades, plus a 5-CNY minimum flat fee.  Use a
                        // 0.5% ceiling with a 50-yuan absolute floor to catch OCR misreads
                        // where a spurious digit is prepended (e.g., "354.57" for "34.57").
                        let commission_cap = 50.0_f64.max(total * 0.005);
                        let commission = if commission_raw > commission_cap {
                            // Try to recover the true commission from the net transaction
                            // amount: THS shows a net figure for sell trades, so
                            //   commission = price×shares − net_amount (= expected − total).
                            let implied = expected - total;
                            if implied > 0.0 && implied < commission_cap {
                                implied
                            } else {
                                // Cannot recover automatically; surface the raw value so
                                // the user can spot and correct it.
                                commission_raw
                            }
                        } else {
                            commission_raw
                        };
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
pub(super) fn pick_fields_combinatorial(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
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
                    let commission_raw = numbers
                        .iter()
                        .enumerate()
                        .filter(|(idx, _)| *idx != i && *idx != j && *idx != k)
                        .map(|(_, &v)| v)
                        .find(|&v| v >= 0.0)
                        .unwrap_or(0.0);
                    let commission_cap = 50.0_f64.max(total * 0.005);
                    let commission = if commission_raw > commission_cap {
                        let implied = expected_total - total;
                        if implied > 0.0 && implied < commission_cap {
                            implied
                        } else {
                            commission_raw
                        }
                    } else {
                        commission_raw
                    };
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
pub(super) fn pick_fields_no_total(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
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
pub(super) fn pick_fields(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    assign_fields_ordered(numbers)
}
