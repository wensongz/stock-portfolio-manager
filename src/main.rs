use regex;

/// One parsed trade row extracted from a 同花顺 (THS) screenshot.
#[derive(Debug, Clone)]
pub struct ParsedTradeRow {
    pub transaction_type: String,
    pub stock_name: String,
    pub traded_at: String,
    pub price: f64,
    pub shares: f64,
    pub total_amount: f64,
    pub commission: f64,
}

fn parse_ths_ocr(text: &str) -> Vec<ParsedTradeRow> {
    let year = extract_year(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut rows: Vec<ParsedTradeRow> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if let Some((tx_type, stock_name, anchor_extra)) = detect_trade_anchor(line) {
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
        } else {
            i += 1;
        }
    }

    rows.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));
    rows.dedup_by(|a, b| a.traded_at == b.traded_at && a.stock_name == b.stock_name);
    rows
}

fn extract_year(text: &str) -> i32 {
    let re = regex::Regex::new(r"(?m)^\s*(\d{4})-\d{2}\s*$").unwrap();
    if let Some(cap) = re.captures(text) {
        if let Ok(y) = cap[1].parse::<i32>() { return y; }
    }
    let re2 = regex::Regex::new(r"\b(\d{4})-\d{2}\b").unwrap();
    if let Some(cap) = re2.captures(text) {
        if let Ok(y) = cap[1].parse::<i32>() { return y; }
    }
    2025
}

fn is_trade_anchor(line: &str) -> bool {
    (line.contains("买入") || line.contains("卖出"))
        && !line.starts_with("类型")
        && !line.starts_with("交易类型")
        && !line.starts_with("方向")
}

fn detect_trade_anchor(line: &str) -> Option<(String, String, String)> {
    if !is_trade_anchor(line) { return None; }
    let tx_type = if line.contains("卖出") { "SELL" } else { "BUY" };
    let without_keyword = line.replace("卖出", " ").replace("买入", " ");
    let stock_name = extract_longest_cjk_run(&without_keyword)?;
    let anchor_extra = without_keyword.replace(&stock_name as &str, " ");
    Some((tx_type.to_string(), stock_name, anchor_extra))
}

fn parse_trade_header(line: &str) -> Option<(String, String)> {
    detect_trade_anchor(line).map(|(tx, name, _)| (tx, name))
}

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
    if current.len() > best.len() { best = current; }
    if best.len() >= 2 && best.len() <= 12 { Some(best) } else { None }
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{f900}'..='\u{faff}'
    )
}

fn extract_fields_from_context(
    tx_type: &str, stock_name: &str, year: i32, anchor_extra: &str, window: &[&str],
) -> Option<ParsedTradeRow> {
    let date_re = regex::Regex::new(r"\b(\d{1,2})-(\d{2})\b").unwrap();
    let time_re = regex::Regex::new(r"\b(\d{1,2}):(\d{2})(?::\d{2})?\b").unwrap();
    let neg_re  = regex::Regex::new(r"-\s*\d+(?:[.,]\d+)?").unwrap();
    let pct_re  = regex::Regex::new(r"\d+(?:\.\d+)?\s*%").unwrap();
    let num_re  = regex::Regex::new(r"\b(\d+(?:\.\d+)?)\b").unwrap();

    let mut parts: Vec<&str> = vec![anchor_extra];
    parts.extend_from_slice(window);
    let all_text = parts.join(" ");

    let (month, day) = date_re.captures(&all_text).map(|c| (
        c[1].parse::<u32>().unwrap_or(1),
        c[2].parse::<u32>().unwrap_or(1),
    ))?;

    let (hour, minute) = time_re.captures(&all_text).map(|c| (
        c[1].parse::<u32>().unwrap_or(9),
        c[2].parse::<u32>().unwrap_or(30),
    )).unwrap_or((9, 30));

    let traded_at = format!("{:04}-{:02}-{:02}T{:02}:{:02}:00", year, month, day, hour, minute);

    let full_date_re = regex::Regex::new(r"\b\d{4}-\d{2}-\d{2}\b").unwrap();
    let cleaned = full_date_re.replace_all(&all_text, " ");
    let cleaned = date_re.replace_all(&cleaned, " ");
    let cleaned = time_re.replace_all(&cleaned, " ");
    let cleaned = neg_re.replace_all(&cleaned, " ");
    let cleaned = pct_re.replace_all(&cleaned, " ");

    let numbers: Vec<f64> = num_re.captures_iter(&cleaned)
        .filter_map(|c| c[1].parse::<f64>().ok())
        .filter(|&n| n > 0.0)
        .collect();

    let (price, shares, total_amount, commission) = assign_fields_ordered(&numbers)?;

    Some(ParsedTradeRow {
        transaction_type: tx_type.to_string(),
        stock_name: stock_name.to_string(),
        traded_at, price, shares, total_amount, commission,
    })
}

const TOTAL_MATCH_TOLERANCE: f64 = 0.02;

fn assign_fields_ordered(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if numbers.is_empty() { return None; }

    let ordered_search = |min_shares: f64| -> Option<(f64, f64, f64, f64)> {
        for pi in 0..numbers.len() {
            let price = numbers[pi];
            if price <= 0.0 || price > 10_000.0 { continue; }
            for si in (pi + 1)..numbers.len() {
                let shares_raw = numbers[si];
                if shares_raw < min_shares || (shares_raw - shares_raw.round()).abs() > 0.5 { continue; }
                let shares = shares_raw.round();
                let expected = price * shares;
                if expected <= 0.0 { continue; }
                for ti in (si + 1)..numbers.len() {
                    let total = numbers[ti];
                    if total <= 0.0 { continue; }
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

    if let Some(r) = ordered_search(100.0) { return Some(r); }
    if let Some(r) = ordered_search(1.0) { return Some(r); }
    pick_fields_combinatorial(numbers)
}

fn pick_fields_combinatorial(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if numbers.len() < 4 { return None; }
    let n = numbers.len().min(8);
    for i in 0..n {
        for j in 0..n {
            if j == i { continue; }
            let price = numbers[i];
            let shares = numbers[j];
            if shares < 1.0 || price <= 0.0 { continue; }
            let expected_total = price * shares;
            for k in 0..n {
                if k == i || k == j { continue; }
                let total = numbers[k];
                if total <= 0.0 { continue; }
                let rel_err = (expected_total - total).abs() / total;
                if rel_err < TOTAL_MATCH_TOLERANCE {
                    let commission = numbers.iter().enumerate()
                        .filter(|(idx, _)| *idx != i && *idx != j && *idx != k)
                        .map(|(_, &v)| v).find(|&v| v >= 0.0).unwrap_or(0.0);
                    return Some((price, shares, total, commission));
                }
            }
        }
    }
    None
}

fn pick_fields(numbers: &[f64]) -> Option<(f64, f64, f64, f64)> {
    assign_fields_ordered(numbers)
}

fn main() {
    run_tests();
    println!("All tests passed!");
}

fn run_tests() {
    // test_extract_year
    assert_eq!(extract_year("2026-04"), 2026);
    assert_eq!(extract_year("foo\n2025-12\nbar"), 2025);
    println!("extract_year: OK");

    // test_parse_trade_header
    let (tx, name) = parse_trade_header("买入-贵州茅台").unwrap();
    assert_eq!(tx, "BUY"); assert_eq!(name, "贵州茅台");
    let (tx, name) = parse_trade_header("卖出 招商银行").unwrap();
    assert_eq!(tx, "SELL"); assert_eq!(name, "招商银行");
    assert!(parse_trade_header("2026-04").is_none());
    println!("parse_trade_header: OK");

    // test_detect_trade_anchor_keyword_at_end
    let (tx, name, extra) = detect_trade_anchor("双汇发展 卖出").unwrap();
    assert_eq!(tx, "SELL"); assert_eq!(name, "双汇发展");
    assert!(!extra.contains("双汇发展"));
    println!("detect_trade_anchor (keyword at end): OK");

    // test_detect_trade_anchor_with_numbers
    let (tx, name, extra) = detect_trade_anchor("卖出-双汇发展  28.41  -56786.02").unwrap();
    assert_eq!(tx, "SELL"); assert_eq!(name, "双汇发展");
    assert!(extra.contains("28.41"));
    assert!(!extra.contains("双汇发展"));
    println!("detect_trade_anchor (with numbers): OK");

    // test_assign_fields_real_case
    let nums = vec![28.41f64, 2000.0, 56820.0, 33.98];
    let (price, shares, total, comm) = assign_fields_ordered(&nums).unwrap();
    assert!((price - 28.41).abs() < 0.01, "price={price}");
    assert!((shares - 2000.0).abs() < 0.01, "shares={shares}");
    assert!((total - 56820.0).abs() < 1.0, "total={total}");
    assert!((comm - 33.98).abs() < 0.01, "comm={comm}");
    println!("assign_fields real case: OK");

    // test_assign_fields_with_rogue_prefix
    let nums = vec![1.0f64, 28.41, 2000.0, 56820.0, 33.98];
    let (price, shares, total, comm) = assign_fields_ordered(&nums).unwrap();
    assert!((price - 28.41).abs() < 0.01, "price={price}");
    assert!((shares - 2000.0).abs() < 0.01, "shares={shares}");
    assert!((total - 56820.0).abs() < 1.0, "total={total}");
    assert!((comm - 33.98).abs() < 0.01, "comm={comm}");
    println!("assign_fields with rogue prefix: OK");

    // test_pick_fields_basic
    let nums = vec![1505.0f64, 100.0, 150500.0, 5.0];
    let (price, shares, total, comm) = pick_fields(&nums).unwrap();
    assert!((price - 1505.0).abs() < 0.01);
    assert!((shares - 100.0).abs() < 0.01);
    assert!((total - 150500.0).abs() < 1.0);
    assert!((comm - 5.0).abs() < 0.01);
    println!("pick_fields basic: OK");

    // test_parse_ths_ocr_single_trade
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
    println!("parse_ths_ocr single trade: OK");

    // test_parse_ths_ocr_multiple_trades_sorted
    let text = "2026-04\n买入-贵州茅台\n04-10  10:00   1505.00  100  150500.00  5.00\n卖出-招商银行\n04-03  14:00   38.50  500  19250.00  3.00\n";
    let rows = parse_ths_ocr(text);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].traded_at < rows[1].traded_at);
    println!("parse_ths_ocr multiple sorted: OK");

    // test keyword NOT at line start (real THS layout)
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
    println!("parse_ths_ocr keyword not at start: OK");

    // test inline format (all fields on one OCR line)
    let text = "2026-04\n卖出双汇发展 04-09 09:58 28.41 2000 56820.00 33.98\n买入招商银行 04-22 14:26 28.95 2000 57900.00 150.00\n";
    let rows = parse_ths_ocr(text);
    assert_eq!(rows.len(), 2, "expected 2 inline rows, got {}: {rows:?}", rows.len());
    assert_eq!(rows[0].stock_name, "双汇发展");
    assert!((rows[0].price - 28.41).abs() < 0.01, "inline sell price={}", rows[0].price);
    assert!((rows[1].price - 28.95).abs() < 0.01, "inline buy price={}", rows[1].price);
    println!("parse_ths_ocr inline format: OK");
}
