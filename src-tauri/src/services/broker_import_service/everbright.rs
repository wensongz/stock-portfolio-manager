use super::{checked_paths, round, rows_to_csv, validate_rows, BrokerTransactionRow};
use encoding_rs::GBK;
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

static LEDGER_SYMBOL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{5,6})$").unwrap());
static LEDGER_CODE_SUFFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{5,6}$").unwrap());
static NAME_ACTION_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i:XD|XR|DR)").unwrap());
static NAME_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^N(?P<c>[\p{Han}])").unwrap());
static BOND_SUFFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:发债|转债|转)$").unwrap());
static SIX_DIGIT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{6}$").unwrap());
static SH_SYMBOL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[5679]|^(110|111|113|118)").unwrap());
static SZ_SYMBOL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0123]").unwrap());
static SH_CONVERTIBLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(110|111|113|118)\d{3}$").unwrap());
static COMPACT_DATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{8}$").unwrap());
static SH_SUBSCRIPTION_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(718|754)").unwrap());

struct LedgerRow {
    values: HashMap<String, String>,
    account: String,
    source_order: usize,
}

fn ledger_value<'a>(row: &'a LedgerRow, key: &str) -> &'a str {
    row.values.get(key).map(String::as_str).unwrap_or("")
}

fn parse_ledger(
    path: &Path,
    account: &str,
    order_base: &mut usize,
) -> Result<Vec<LedgerRow>, String> {
    let bytes = fs::read(path).map_err(|e| format!("读取 {} 失败：{}", path.display(), e))?;
    let (decoded, _, had_errors) = GBK.decode(&bytes);
    if had_errors {
        return Err(format!("文件不是有效的 GBK 对账单：{}", path.display()));
    }
    let mut lines = decoded
        .trim_start_matches('\u{feff}')
        .lines()
        .filter(|line| !line.is_empty());
    let headers: Vec<String> = lines
        .next()
        .ok_or_else(|| format!("文件为空：{}", path.display()))?
        .split('\t')
        .map(str::to_string)
        .collect();
    if !headers.iter().any(|h| h == "摘要") || !headers.iter().any(|h| h == "发生日期") {
        return Err(format!("无法识别光大对账单表头：{}", path.display()));
    }

    let mut rows = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').collect();
        let values = headers
            .iter()
            .enumerate()
            .map(|(index, header)| {
                (
                    header.clone(),
                    cells.get(index).copied().unwrap_or("").to_string(),
                )
            })
            .collect();
        rows.push(LedgerRow {
            values,
            account: account.to_string(),
            source_order: *order_base,
        });
        *order_base += 1;
    }
    Ok(rows)
}

fn ledger_num(value: &str) -> f64 {
    value
        .trim()
        .trim_start_matches("=\"")
        .trim_end_matches('"')
        .replace(',', "")
        .parse()
        .unwrap_or(0.0)
}

fn ledger_symbol(summary: &str) -> String {
    LEDGER_SYMBOL_RE
        .captures(summary)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_default()
}

fn ledger_event_type(summary: &str) -> String {
    LEDGER_CODE_SUFFIX_RE.replace(summary, "").to_string()
}

fn clean_ledger_name(value: &str) -> String {
    let without_action = NAME_ACTION_RE.replace(value.trim(), "");
    NAME_PREFIX_RE
        .replace(&without_action, "$c")
        .trim()
        .to_string()
}

fn base_bond_name(value: &str) -> String {
    BOND_SUFFIX_RE
        .replace(&clean_ledger_name(value), "")
        .trim()
        .to_string()
}

fn ledger_market(symbol: &str) -> &'static str {
    if symbol.len() == 5 {
        "HK"
    } else {
        "CN"
    }
}

fn canonical_symbol(symbol: &str, market: &str) -> Result<String, String> {
    if market != "CN" || !SIX_DIGIT_RE.is_match(symbol) {
        return Ok(symbol.to_string());
    }
    if SH_SYMBOL_RE.is_match(symbol) {
        Ok(format!("sh{}", symbol))
    } else if SZ_SYMBOL_RE.is_match(symbol) {
        Ok(format!("sz{}", symbol))
    } else {
        Err(format!("无法判断 A 股交易所前缀：{}", symbol))
    }
}

fn is_sh_convertible(symbol: &str) -> bool {
    SH_CONVERTIBLE_RE.is_match(symbol)
}

fn ledger_fees(row: &LedgerRow) -> f64 {
    round(
        ["佣金", "过户费", "印花税", "其他费"]
            .iter()
            .map(|key| ledger_num(ledger_value(row, key)).abs())
            .sum(),
        3,
    )
}

fn ledger_date(value: &str) -> String {
    let value = value.trim();
    if COMPACT_DATE_RE.is_match(value) {
        format!("{}-{}-{}", &value[0..4], &value[4..6], &value[6..8])
    } else {
        value.to_string()
    }
}

fn push_ledger_row(
    output: &mut Vec<BrokerTransactionRow>,
    row: &LedgerRow,
    symbol: &str,
    name: &str,
    market: &str,
    tx_type: &str,
    shares: f64,
    price: f64,
    total_amount: Option<f64>,
    commission: f64,
    note: &str,
) -> Result<(), String> {
    let order_no = ledger_value(row, "委托编号").trim();
    let mut notes = row.account.clone();
    if !order_no.is_empty() {
        notes.push_str(&format!("；委托编号 {}", order_no));
    }
    if !note.is_empty() {
        notes.push_str(&format!("；{}", note));
    }
    output.push(BrokerTransactionRow {
        traded_at: ledger_date(ledger_value(row, "发生日期")),
        symbol: canonical_symbol(symbol, market)?,
        name: name.to_string(),
        market: market.to_string(),
        transaction_type: tx_type.to_string(),
        shares,
        price,
        total_amount,
        commission,
        currency: "CNY".to_string(),
        notes,
        source_order: row.source_order,
    });
    Ok(())
}

pub fn convert_everbright(
    ordinary_files: Vec<String>,
    credit_files: Vec<String>,
    supplement_files: Vec<String>,
) -> Result<String, String> {
    let ordinary = checked_paths(ordinary_files, "xls")?;
    let credit = checked_paths(credit_files, "xls")?;
    let supplements = checked_paths(supplement_files, "xls")?;
    if ordinary.len() + credit.len() <= 1 {
        return Err("普通账户与信用账户主对账单合计必须至少上传 2 个文件".to_string());
    }
    if !supplements.is_empty() && ordinary.is_empty() {
        return Err("上传普通账户补充记录时，必须同时上传普通账户主对账单".to_string());
    }

    let mut order = 0usize;
    let mut rows = Vec::new();
    for path in ordinary.iter().chain(supplements.iter()) {
        rows.extend(parse_ledger(path, "普通账户", &mut order)?);
    }
    for path in &credit {
        rows.extend(parse_ledger(path, "信用账户", &mut order)?);
    }

    let mut name_votes: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for row in &rows {
        let symbol = ledger_symbol(ledger_value(row, "摘要"));
        let name = clean_ledger_name(ledger_value(row, "证券名称"));
        if !symbol.is_empty() && !name.is_empty() {
            *name_votes
                .entry(symbol)
                .or_default()
                .entry(name)
                .or_default() += 1;
        }
    }
    let code_name: HashMap<String, String> = name_votes
        .into_iter()
        .map(|(symbol, votes)| {
            let mut votes: Vec<(String, usize)> = votes.into_iter().collect();
            votes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            (symbol, votes[0].0.clone())
        })
        .collect();

    let mut listed_bond_code = HashMap::new();
    for row in &rows {
        if ledger_event_type(ledger_value(row, "摘要")) == "上市流通" {
            let symbol = ledger_symbol(ledger_value(row, "摘要"));
            let base = base_bond_name(ledger_value(row, "证券名称"));
            if !symbol.is_empty() && !base.is_empty() {
                listed_bond_code.insert(base, symbol);
            }
        }
    }

    let mut output = Vec::new();
    for row in &rows {
        let event = ledger_event_type(ledger_value(row, "摘要"));
        let symbol = ledger_symbol(ledger_value(row, "摘要"));
        let name = {
            let raw = clean_ledger_name(ledger_value(row, "证券名称"));
            if raw.is_empty() {
                code_name.get(&symbol).cloned().unwrap_or_default()
            } else {
                raw
            }
        };
        let amount = ledger_num(ledger_value(row, "发生金额"));
        let fees = ledger_fees(row);
        let raw_shares = ledger_num(ledger_value(row, "成交数量")).abs();
        let raw_price = ledger_num(ledger_value(row, "成交均价")).abs();

        if ["证券买入", "证券卖出", "融资买入"].contains(&event.as_str()) {
            let is_buy = event != "证券卖出";
            let shares = if is_sh_convertible(&symbol) {
                raw_shares * 10.0
            } else {
                raw_shares
            };
            push_ledger_row(
                &mut output,
                row,
                &symbol,
                &name,
                ledger_market(&symbol),
                if is_buy { "BUY" } else { "SELL" },
                round(shares, 6),
                round(raw_price, 6),
                None,
                fees,
                if event == "融资买入" {
                    "融资买入"
                } else {
                    "证券成交"
                },
            )?;
            continue;
        }

        if ["港股待交收买入成交", "港股待交收卖出成交"].contains(&event.as_str())
        {
            let is_buy = event.contains("买入");
            let gross_cny = if is_buy {
                amount.abs() - fees
            } else {
                amount + fees
            };
            let price = if raw_shares > 0.0 {
                gross_cny / raw_shares
            } else {
                0.0
            };
            push_ledger_row(
                &mut output,
                row,
                &symbol,
                &name,
                "HK",
                if is_buy { "BUY" } else { "SELL" },
                round(raw_shares, 6),
                round(price, 6),
                None,
                fees,
                &format!("港股通人民币结算；原始港币报价 {}", round(raw_price, 6)),
            )?;
            continue;
        }

        if event == "LOF申购" {
            let gross = (amount.abs() - fees).max(0.0);
            push_ledger_row(
                &mut output,
                row,
                &symbol,
                &name,
                ledger_market(&symbol),
                "BUY",
                round(raw_shares, 6),
                round(
                    if raw_shares > 0.0 {
                        gross / raw_shares
                    } else {
                        0.0
                    },
                    6,
                ),
                None,
                fees,
                "LOF场外申购，人民币成交价由净扣款还原",
            )?;
            continue;
        }

        if event == "申购中签(转非流通)" {
            let base = base_bond_name(&name);
            let actual = listed_bond_code
                .get(&base)
                .cloned()
                .unwrap_or(symbol.clone());
            let shares = if is_sh_convertible(&actual) || SH_SUBSCRIPTION_RE.is_match(&symbol) {
                raw_shares * 10.0
            } else {
                raw_shares
            };
            let actual_name = code_name
                .get(&actual)
                .cloned()
                .unwrap_or_else(|| name.trim_end_matches("发债").to_string() + "转债");
            let note = if actual == symbol {
                "可转债中签（尚无上市代码）".to_string()
            } else {
                format!("可转债中签；原申购代码 {}", symbol)
            };
            push_ledger_row(
                &mut output,
                row,
                &actual,
                &actual_name,
                ledger_market(&actual),
                "BUY",
                round(shares, 6),
                100.0,
                None,
                0.0,
                &note,
            )?;
            continue;
        }

        let pay_note = match event.as_str() {
            "股息入帐" => Some((ledger_market(&symbol), "现金分红")),
            "港股通红利发放" => Some(("HK", "港股通现金分红（人民币）")),
            "股息红利扣税" => Some((
                ledger_market(&symbol),
                "红利税（负数，保留用于完整收益与成本核算）",
            )),
            "IFC申购扣款" => Some((ledger_market(&symbol), "基金场外申购扣款（源流水无份额）")),
            "IFC赎回返款" => Some((ledger_market(&symbol), "基金场外赎回返款（源流水无份额）")),
            _ => None,
        };
        if let Some((market, note)) = pay_note {
            push_ledger_row(
                &mut output,
                row,
                &symbol,
                &name,
                market,
                "PAY",
                0.0,
                0.0,
                Some(round(amount, 3)),
                0.0,
                note,
            )?;
        }
    }

    output.sort_by(|a, b| {
        a.traded_at
            .cmp(&b.traded_at)
            .then_with(|| a.source_order.cmp(&b.source_order))
    });
    validate_rows(&output, Some("CNY"))?;
    rows_to_csv(&output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_cn_symbols() {
        assert_eq!(canonical_symbol("600938", "CN").unwrap(), "sh600938");
        assert_eq!(canonical_symbol("000807", "CN").unwrap(), "sz000807");
        assert_eq!(canonical_symbol("00883", "HK").unwrap(), "00883");
    }
}
