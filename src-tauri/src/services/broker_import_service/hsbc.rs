use super::{checked_paths, round, rows_to_csv, validate_rows, BrokerTransactionRow};
use chrono::NaiveDate;
use regex::Regex;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone)]
struct HsbcTrade {
    source_file: String,
    source_page: usize,
    statement_date: String,
    symbol: String,
    name: String,
    transaction_date: String,
    settlement_date: Option<String>,
    unit_price: Option<f64>,
    quantity: f64,
    quantity_out: bool,
    settlement_amount: Option<f64>,
    reference: String,
    transaction_type: String,
}

#[derive(Debug, Clone)]
struct HsbcBenefit {
    date: String,
    symbol: String,
    reference: String,
    amount: f64,
}

#[derive(Debug, Clone)]
struct HsbcStatement {
    trades: Vec<HsbcTrade>,
    benefits: Vec<HsbcBenefit>,
    unmatched_lines: Vec<String>,
}

fn hsbc_date(value: &str) -> Result<String, String> {
    NaiveDate::parse_from_str(value, "%d%b%Y")
        .map(|date| date.format("%Y-%m-%d").to_string())
        .map_err(|_| format!("无法识别汇丰日期：{}", value))
}

fn hsbc_number(value: Option<&str>) -> Option<f64> {
    value.and_then(|v| v.replace(',', "").parse().ok())
}

fn extract_hsbc_statement(path: &Path) -> Result<HsbcStatement, String> {
    let document = pdf_oxide::PdfDocument::open(path)
        .map_err(|e| format!("读取汇丰 PDF {} 失败：{}", path.display(), e))?;
    let page_count = document
        .page_count()
        .map_err(|e| format!("读取汇丰 PDF {} 页数失败：{}", path.display(), e))?;
    let pages = (0..page_count)
        .map(|page_index| {
            document.extract_text(page_index).map_err(|e| {
                format!(
                    "读取汇丰 PDF {} 第{}页失败：{}",
                    path.display(),
                    page_index + 1,
                    e
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let combined = pages.join("\n");
    let statement_date_re = Regex::new(r"Date\s*:\s*(\d{2}[A-Z]{3}\d{4})").unwrap();
    let statement_date = statement_date_re
        .captures(&combined)
        .and_then(|c| c.get(1))
        .map(|value| value.as_str().to_string())
        .or_else(|| {
            Regex::new(r"_(\d{8})-(\d{8})\.pdf$")
                .unwrap()
                .captures(&path.to_string_lossy())
                .map(|captures| {
                    let raw = &captures[2];
                    let date = NaiveDate::parse_from_str(raw, "%Y%m%d").ok()?;
                    Some(date.format("%d%b%Y").to_string().to_uppercase())
                })
                .flatten()
        })
        .ok_or_else(|| format!("无法识别结单日期：{}", path.display()))?;
    let statement_date = hsbc_date(&statement_date)?;
    let security_re = Regex::new(r"^(\d{5})\s*(.+?)(?:\s+\((?:SHS|UNT)\))?\s*$").unwrap();
    let trade_re = Regex::new(
        r"^(\d{2}[A-Z]{3}\d{4})\s*(\d{2}[A-Z]{3}\d{4}|TBC)\s*(HKD|N/A)(?:\s*([\d,]+\.\d{4}))?\s*([\d,]+)(-)?(?:\s*HKD\s*([\d,.]+))?\s*$",
    )
    .unwrap();
    let reference_re = Regex::new(r"^Reference:\s*(\S+?)\s*Type:\s*(\S+)(.*)$").unwrap();
    let mut trades = Vec::new();
    let mut unmatched_lines = Vec::new();
    let mut last_security: HashMap<String, (String, String)> = HashMap::new();

    let charge_event_re = Regex::new(
        r"(?m)^\d{2}[A-Z]{3}\d{4}(?:PURCHASE|SALE|COVERED WARRANT CONVERSION)(\d{5})\s*$",
    )
    .unwrap();
    let charge_reference_re = Regex::new(r"(?m)^OUR REFERENCE:(\S+)\s*$").unwrap();
    let charges = pages
        .iter()
        .filter_map(|page| {
            page.split_once("Charges and income summary")
                .map(|(_, value)| value)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let symbols: Vec<String> = charge_event_re
        .captures_iter(&charges)
        .map(|captures| captures[1].to_string())
        .collect();
    let references: Vec<String> = charge_reference_re
        .captures_iter(&charges)
        .map(|captures| captures[1].to_string())
        .collect();
    let mut reference_symbols = HashMap::new();
    for (reference, symbol) in references.into_iter().zip(symbols) {
        reference_symbols.insert(reference, symbol);
    }

    let mut security_names = HashMap::new();
    for line in combined.lines().map(str::trim) {
        if let Some(captures) = security_re.captures(line) {
            security_names
                .entry(captures[1].to_string())
                .or_insert_with(|| captures[2].trim().to_string());
        }
    }

    for (page_index, page) in pages.iter().enumerate() {
        let mut in_transactions = false;
        let mut category = String::new();
        let mut symbol = String::new();
        let mut name = String::new();
        let mut pending: Option<HsbcTrade> = None;
        for line in page.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if line == "Transaction summary" {
                in_transactions = true;
                category.clear();
                symbol.clear();
                name.clear();
                pending = None;
                continue;
            }
            if line == "Charges and income summary" {
                continue;
            }
            if !in_transactions {
                continue;
            }
            if ["LOCAL SHARES", "WARRANTS", "OTHERS"].contains(&line) {
                category = line.to_string();
                if let Some((last_symbol, last_name)) = last_security.get(line) {
                    symbol = last_symbol.clone();
                    name = last_name.clone();
                }
                continue;
            }
            if line.starts_with("Securities Securities description")
                || line.starts_with("ID Transaction date")
                || line == "/Settlement date"
            {
                continue;
            }
            if let Some(captures) = security_re.captures(line) {
                symbol = captures[1].to_string();
                name = captures[2].to_string();
                last_security.insert(category.clone(), (symbol.clone(), name.clone()));
                continue;
            }
            if let Some(captures) = trade_re.captures(line) {
                if symbol.is_empty() {
                    unmatched_lines.push(format!(
                        "{} 第{}页：{}",
                        path.display(),
                        page_index + 1,
                        line
                    ));
                    continue;
                }
                pending = Some(HsbcTrade {
                    source_file: path
                        .file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    source_page: page_index + 1,
                    statement_date: statement_date.clone(),
                    symbol: symbol.clone(),
                    name: name.clone(),
                    transaction_date: hsbc_date(&captures[1])?,
                    settlement_date: if &captures[2] == "TBC" {
                        None
                    } else {
                        Some(hsbc_date(&captures[2])?)
                    },
                    unit_price: hsbc_number(captures.get(4).map(|m| m.as_str())),
                    quantity: hsbc_number(captures.get(5).map(|m| m.as_str())).unwrap_or(0.0),
                    quantity_out: captures.get(6).is_some(),
                    settlement_amount: hsbc_number(captures.get(7).map(|m| m.as_str())),
                    reference: String::new(),
                    transaction_type: String::new(),
                });
                continue;
            }
            if let Some(captures) = reference_re.captures(line) {
                if let Some(mut trade) = pending.take() {
                    trade.reference = captures[1].to_string();
                    trade.transaction_type = captures[2].to_string();
                    if let Some(actual_symbol) = reference_symbols.get(&trade.reference) {
                        trade.symbol = actual_symbol.clone();
                        if let Some(actual_name) = security_names.get(actual_symbol) {
                            trade.name = actual_name.clone();
                        }
                    } else if trade.settlement_date.is_none() {
                        continue;
                    }
                    trades.push(trade);
                }
            }
        }
    }

    let benefit_re = Regex::new(
        r"(?s)(\d{2}[A-Z]{3}\d{4})\s*COVERED WARRANT CONVERSION\s*(\d{5}).*?OUR REFERENCE:\s*(\S+).*?PAID BENEFITS\s*HKD\s*([\d,.]+)",
    )
    .unwrap();
    let benefits = benefit_re
        .captures_iter(&combined)
        .map(|c| {
            Ok(HsbcBenefit {
                date: hsbc_date(&c[1])?,
                symbol: c[2].to_string(),
                reference: c[3].to_string(),
                amount: hsbc_number(Some(&c[4])).unwrap_or(0.0),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(HsbcStatement {
        trades,
        benefits,
        unmatched_lines,
    })
}

fn same_hsbc_trade(a: &HsbcTrade, b: &HsbcTrade) -> bool {
    a.symbol == b.symbol
        && a.transaction_date == b.transaction_date
        && a.unit_price == b.unit_price
        && a.quantity == b.quantity
        && a.quantity_out == b.quantity_out
        && a.settlement_amount == b.settlement_amount
        && a.transaction_type == b.transaction_type
}

fn clean_hsbc_name(value: &str) -> String {
    let mut value = Regex::new(r"^(?i:UNTRADE-)")
        .unwrap()
        .replace(value, "")
        .trim()
        .to_string();
    if value.to_uppercase().ends_with("(CBBC") {
        value.push(')');
    }
    value
}

pub fn convert_hsbc(files: Vec<String>) -> Result<String, String> {
    let files = checked_paths(files, "pdf")?;
    if files.is_empty() {
        return Err("请至少上传一份汇丰电子结单".to_string());
    }
    let statements = files
        .iter()
        .map(|path| extract_hsbc_statement(path))
        .collect::<Result<Vec<_>, _>>()?;
    let unmatched: Vec<&String> = statements
        .iter()
        .flat_map(|statement| statement.unmatched_lines.iter())
        .collect();
    if !unmatched.is_empty() {
        return Err(format!(
            "有 {} 条汇丰交易行未识别，示例：{}",
            unmatched.len(),
            unmatched[0]
        ));
    }

    let mut by_reference: BTreeMap<String, Vec<HsbcTrade>> = BTreeMap::new();
    for trade in statements
        .iter()
        .flat_map(|statement| statement.trades.iter())
    {
        by_reference
            .entry(trade.reference.clone())
            .or_default()
            .push(trade.clone());
    }
    let mut unique = Vec::new();
    for (reference, mut records) in by_reference {
        if records
            .iter()
            .skip(1)
            .any(|record| !same_hsbc_trade(&records[0], record))
        {
            return Err(format!("汇丰参考号 {} 在不同结单中的内容不一致", reference));
        }
        records.sort_by(|a, b| {
            b.settlement_date
                .is_some()
                .cmp(&a.settlement_date.is_some())
                .then_with(|| b.statement_date.cmp(&a.statement_date))
        });
        unique.push(records.remove(0));
    }
    unique.sort_by(|a, b| {
        a.transaction_date
            .cmp(&b.transaction_date)
            .then_with(|| a.reference.cmp(&b.reference))
    });

    let mut seen_benefits = HashSet::new();
    let mut benefits: HashMap<(String, String), f64> = HashMap::new();
    for benefit in statements
        .iter()
        .flat_map(|statement| statement.benefits.iter())
    {
        if seen_benefits.insert(benefit.reference.clone()) {
            *benefits
                .entry((benefit.date.clone(), benefit.symbol.clone()))
                .or_default() += benefit.amount;
        }
    }

    let mut output = Vec::new();
    for (index, trade) in unique.into_iter().enumerate() {
        if !Regex::new(r"^\d{5}$").unwrap().is_match(&trade.symbol) {
            return Err(format!("港股代码格式异常：{}", trade.symbol));
        }
        if !["PUR", "SAL", "CWN"].contains(&trade.transaction_type.as_str()) {
            return Err(format!("不支持的汇丰交易类型：{}", trade.transaction_type));
        }
        if trade.quantity <= 0.0 {
            return Err(format!("成交数量异常：{}", trade.reference));
        }
        let is_buy = trade.transaction_type == "PUR";
        let is_conversion = trade.transaction_type == "CWN";
        let benefit = if is_conversion {
            benefits
                .get(&(trade.transaction_date.clone(), trade.symbol.clone()))
                .copied()
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let price = if is_conversion {
            round(benefit / trade.quantity, 6)
        } else {
            round(trade.unit_price.unwrap_or(0.0), 6)
        };
        let gross = round(price * trade.quantity, 6);
        let commission = if is_conversion {
            0.0
        } else if is_buy {
            round(trade.settlement_amount.unwrap_or(0.0) - gross, 2)
        } else {
            round(gross - trade.settlement_amount.unwrap_or(0.0), 2)
        };
        if commission < -0.01 {
            return Err(format!("结算金额与交易方向不符：{}", trade.reference));
        }
        let conversion_note = if is_conversion {
            if benefit > 0.0 {
                format!("；权证到期转换，派付权益 HKD {:.2}", benefit)
            } else {
                "；权证到期转换，无派付权益".to_string()
            }
        } else {
            String::new()
        };
        let settlement = trade.settlement_date.as_deref().unwrap_or("结单时待定");
        output.push(BrokerTransactionRow {
            traded_at: trade.transaction_date,
            symbol: trade.symbol,
            name: clean_hsbc_name(&trade.name),
            market: "HK".to_string(),
            transaction_type: if is_buy { "BUY" } else { "SELL" }.to_string(),
            shares: round(trade.quantity, 6),
            price,
            total_amount: None,
            commission: commission.max(0.0),
            currency: "HKD".to_string(),
            notes: format!(
                "汇丰证券账户；参考号 {}；结算日 {}{}；来源 {} 第{}页",
                trade.reference, settlement, conversion_note, trade.source_file, trade.source_page
            ),
            source_order: index,
        });
    }
    output.sort_by(|a, b| {
        a.traded_at
            .cmp(&b.traded_at)
            .then_with(|| a.notes.cmp(&b.notes))
    });
    validate_rows(&output, Some("HKD"))?;
    rows_to_csv(&output)
}
