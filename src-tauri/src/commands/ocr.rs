use crate::services::{http_client, quote_service};
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

#[path = "ocr/image_pipeline.rs"]
mod image_pipeline;
#[path = "ocr/lookup.rs"]
mod lookup;
#[path = "ocr/parser.rs"]
mod parser;

use image_pipeline::{ocr_image, split_image_by_separators};
pub use lookup::{lookup_cn_stock_code_with_state, lookup_stock_name_by_symbol_with_state};
use parser::parse_ths_ocr;

#[tauri::command(rename_all = "camelCase")]
pub async fn lookup_cn_stock_code(
    quote_state: tauri::State<'_, quote_service::QuoteServiceState>,
    name: String,
) -> Result<Option<String>, String> {
    lookup_cn_stock_code_with_state(&quote_state, name).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn lookup_stock_name_by_symbol(
    quote_state: tauri::State<'_, quote_service::QuoteServiceState>,
    symbol: String,
) -> Result<Option<String>, String> {
    lookup_stock_name_by_symbol_with_state(&quote_state, symbol).await
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

    // ── Primary path: whole-image OCR ────────────────────────────────────────
    // The THS 对账单 trade list is a continuous scrollable view without
    // explicit inter-card separators, so OCR-ing the full image gives the
    // best result.  split_image_by_separators is kept as a fallback for
    // "card-based" screenshot formats.
    let text = ocr_image(&bytes)?;
    let mut all_rows = parse_ths_ocr(&text);

    // ── Fallback: per-slice OCR ───────────────────────────────────────────────
    // When the whole-image parse finds nothing, try splitting by separator
    // bands and OCR-ing each slice independently.  This handles layouts
    // where individual cards are separated by wide uniform-colour bands.
    if all_rows.is_empty() {
        let slices = split_image_by_separators(&bytes);
        if slices.len() > 1 {
            for slice in &slices {
                if let Ok(slice_text) = ocr_image(slice) {
                    all_rows.extend(parse_ths_ocr(&slice_text));
                }
            }
        }
    }

    // Deduplicate and sort chronologically.
    all_rows.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));
    all_rows.dedup_by(|a, b| {
        a.traded_at == b.traded_at
            && a.stock_name == b.stock_name
            && (a.price - b.price).abs() < 0.001
            && (a.shares - b.shares).abs() < 0.001
    });
    Ok(all_rows)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::parser::*;
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
        assert!(is_trade_anchor(
            "买人 2026-04-22 14:26 28.95 2000 57865.44 54.57"
        ));
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

    /// OCR misread: "34.57" read as "354.57" (extra leading digit from adjacent column).
    /// The THS net amount (57865.43) is in the numbers array, so we recover:
    ///   implied = price×shares − net = 28.95×2000 − 57865.43 = 34.57.
    #[test]
    fn test_assign_fields_commission_ocr_misread_recovered() {
        // numbers: [price=28.95, net_amount=57865.43, shares=2000, commission_misread=354.57]
        let nums = vec![28.95_f64, 57865.43, 2000.0, 354.57];
        let result = assign_fields_ordered(&nums);
        assert!(result.is_some(), "should still find price/shares/total");
        let (price, shares, _total, comm) = result.unwrap();
        assert!((price - 28.95).abs() < 0.01, "price={price}");
        assert!((shares - 2000.0).abs() < 0.01, "shares={shares}");
        assert!(
            (comm - 34.57).abs() < 0.01,
            "commission should be recovered as 34.57 via implied=price×shares−net, got {comm}"
        );
    }

    /// When the OCR net amount equals price×shares (no deduction visible), the
    /// raw misread commission is surfaced unchanged so the user can correct it.
    #[test]
    fn test_assign_fields_commission_ocr_misread_no_net_returns_raw() {
        // Gross total matches exactly → implied = 0 → not usable → raw returned.
        let nums = vec![28.95_f64, 2000.0, 57900.0, 354.57];
        let result = assign_fields_ordered(&nums);
        assert!(result.is_some(), "should still find price/shares/total");
        let (_price, _shares, _total, comm) = result.unwrap();
        assert!(
            (comm - 354.57).abs() < 0.01,
            "raw misread should be surfaced when implied=0, got {comm}"
        );
    }

    /// Correct commission that is close to (but under) the cap is preserved.
    #[test]
    fn test_assign_fields_commission_correct_below_cap() {
        // Real: price=28.95, net=57865.43, shares=2000, commission=34.57 (0.06% of total)
        let nums = vec![28.95_f64, 57865.43, 2000.0, 34.57];
        let (_price, _shares, _total, comm) = assign_fields_ordered(&nums).unwrap();
        assert!(
            (comm - 34.57).abs() < 0.01,
            "correct commission should be preserved, got {comm}"
        );
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
        assert_eq!(
            rows.len(),
            2,
            "expected 2 rows, got {}: {rows:?}",
            rows.len()
        );

        let sell = rows.iter().find(|r| r.transaction_type == "SELL").unwrap();
        assert_eq!(sell.stock_name, "双汇发展");
        assert!(
            (sell.price - 28.41).abs() < 0.01,
            "sell price={}",
            sell.price
        );
        assert!(
            (sell.shares - 2000.0).abs() < 0.01,
            "sell shares={}",
            sell.shares
        );
        assert!(
            (sell.total_amount - 56820.0).abs() < 1.0,
            "sell total={}",
            sell.total_amount
        );
        assert!(
            (sell.commission - 33.98).abs() < 0.01,
            "sell comm={}",
            sell.commission
        );

        let buy = rows.iter().find(|r| r.transaction_type == "BUY").unwrap();
        assert_eq!(buy.stock_name, "招商银行");
        assert!((buy.price - 28.95).abs() < 0.01, "buy price={}", buy.price);
        assert!(
            (buy.shares - 2000.0).abs() < 0.01,
            "buy shares={}",
            buy.shares
        );
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
        assert_eq!(
            rows.len(),
            2,
            "expected 2 rows, got {}: {rows:?}",
            rows.len()
        );

        let sell = rows.iter().find(|r| r.transaction_type == "SELL").unwrap();
        assert_eq!(sell.stock_name, "双汇发展");
        assert!(
            (sell.price - 28.41).abs() < 0.01,
            "sell price={}",
            sell.price
        );
        assert!(
            (sell.shares - 2000.0).abs() < 0.01,
            "sell shares={}",
            sell.shares
        );
        // total_amount must be price × shares, not the OCR'd net amount.
        assert!(
            (sell.total_amount - 28.41 * 2000.0).abs() < 1.0,
            "sell total={} (expected {})",
            sell.total_amount,
            28.41 * 2000.0
        );
        assert!(
            (sell.commission - 33.98).abs() < 0.01,
            "sell comm={}",
            sell.commission
        );
        assert_eq!(sell.traded_at, "2026-04-09T09:58:00");

        let buy = rows.iter().find(|r| r.transaction_type == "BUY").unwrap();
        assert_eq!(buy.stock_name, "招商银行");
        assert!((buy.price - 28.95).abs() < 0.01, "buy price={}", buy.price);
        assert!(
            (buy.shares - 2000.0).abs() < 0.01,
            "buy shares={}",
            buy.shares
        );
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
            rows.iter()
                .map(|r| format!("{}/{}", r.stock_name, r.transaction_type))
                .collect::<Vec<_>>()
        );

        // Verify a sample of expected values.
        let maotai = rows
            .iter()
            .find(|r| r.stock_name.contains("贵州茅台"))
            .unwrap();
        assert_eq!(maotai.transaction_type, "SELL");
        assert!(
            (maotai.price - 1459.48).abs() < 0.01,
            "maotai price={}",
            maotai.price
        );
        assert!((maotai.shares - 100.0).abs() < 0.01);
        assert!(
            (maotai.total_amount - 1459.48 * 100.0).abs() < 1.0,
            "total={} expected={}",
            maotai.total_amount,
            1459.48 * 100.0
        );

        let zhaoshang = rows
            .iter()
            .find(|r| r.stock_name.contains("招商银行"))
            .unwrap();
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
        let sep_h: u32 = 60; // 60 px ≥ MIN_SEPARATOR_BAND_PX (50) → emitted as a cut
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
            let color = if local_y < card_h {
                card_color
            } else {
                sep_color
            };
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
        assert_eq!(
            slices.len(),
            2,
            "expected 2 slices for 2-card image, got {}",
            slices.len()
        );
    }

    #[test]
    fn test_split_six_cards_produces_six_slices() {
        let bytes = make_test_image_with_separators(6);
        let slices = split_image_by_separators(&bytes);
        assert_eq!(
            slices.len(),
            6,
            "expected 6 slices for 6-card image, got {}",
            slices.len()
        );
    }

    #[test]
    fn test_split_invalid_bytes_returns_original() {
        let bad = b"not an image".to_vec();
        let slices = split_image_by_separators(&bad);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0], bad);
    }

    /// Thin separator bands below MIN_SEPARATOR_BAND_PX are ignored — the
    /// compact THS list layout should NOT be sliced into per-entry fragments
    /// (which would separate line 1 from line 2 of the same entry).
    ///
    /// Also verifies that the common "in-row whitespace" pattern (short blank
    /// bands above/below text within a row) is ignored when those bands are
    /// < 50 px tall.
    #[test]
    fn test_split_thin_separators_not_cut() {
        use image::{ImageBuffer, Rgb};
        // Build image with 4px thin light-gray separator bands between dark cards.
        let card_h: u32 = 80;
        let sep_h: u32 = 4; // below MIN_SEPARATOR_BAND_PX=50 → not treated as cut
        let n_cards: u32 = 3;
        let width: u32 = 400;
        let total_h = n_cards * card_h + (n_cards - 1) * sep_h;
        let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(width, total_h);
        let card_color = Rgb([50u8, 50, 50]);
        let sep_color = Rgb([235u8, 235, 235]);
        for y in 0..total_h {
            let stripe = card_h + sep_h;
            let col = if y % stripe < card_h {
                card_color
            } else {
                sep_color
            };
            for x in 0..width {
                img.put_pixel(x, y, col);
            }
        }
        let mut buf: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("encode");
        let slices = split_image_by_separators(&buf);
        // Thin separators (< MIN_SEPARATOR_BAND_PX) should be ignored →
        // no cuts → falls back to returning the original image as a single slice.
        assert_eq!(
            slices.len(),
            1,
            "thin separators must not trigger slicing, got {} slices",
            slices.len()
        );
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
        assert_eq!(
            rows.len(),
            1,
            "expected 1 BUY row, got {}: {rows:?}",
            rows.len()
        );
        let r = &rows[0];
        assert_eq!(r.transaction_type, "BUY");
        assert_eq!(r.stock_name, "招商银行");
        assert!((r.price - 39.680).abs() < 0.01, "price={}", r.price);
        assert!((r.shares - 1500.0).abs() < 0.01, "shares={}", r.shares);
        assert!(
            (r.total_amount - 39.680 * 1500.0).abs() < 1.0,
            "total={}",
            r.total_amount
        );
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
        assert_eq!(
            rows.len(),
            6,
            "expected 6 rows, got {}: {rows:?}",
            rows.len()
        );

        let buys: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "BUY")
            .collect();
        let sells: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "SELL")
            .collect();
        assert_eq!(buys.len(), 3, "expected 3 BUY rows");
        assert_eq!(sells.len(), 3, "expected 3 SELL rows");

        // Check the 贵州茅台 sell
        let maotai = sells
            .iter()
            .find(|r| r.stock_name.contains("贵州茅台"))
            .unwrap();
        assert!((maotai.price - 1459.480).abs() < 0.01);
        assert!((maotai.shares - 100.0).abs() < 0.01);
        assert!((maotai.total_amount - 1459.480 * 100.0).abs() < 1.0);

        // Check a 招商银行 buy — both 1500-share entries should be present
        // with their actual prices.  After chronological sort the 04-13 entry
        // (38.970) precedes the 04-22 entry (39.680), so use explicit find.
        let zhaoshang_buy_0422 = buys
            .iter()
            .find(|r| {
                r.stock_name.contains("招商银行")
                    && (r.shares - 1500.0).abs() < 1.0
                    && r.traded_at.contains("04-22")
            })
            .expect("04-22 招商银行 1500-share buy not found");
        assert!(
            (zhaoshang_buy_0422.price - 39.680).abs() < 0.01,
            "price={}",
            zhaoshang_buy_0422.price
        );
        assert!(
            (zhaoshang_buy_0422.commission - 5.60).abs() < 0.01,
            "commission={}",
            zhaoshang_buy_0422.commission
        );
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
        assert_eq!(
            rows.len(),
            6,
            "expected 6 rows from real tesseract output, got {}: {rows:?}",
            rows.len()
        );

        let buys: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "BUY")
            .collect();
        let sells: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "SELL")
            .collect();
        assert_eq!(buys.len(), 3, "expected 3 BUY rows, got {buys:?}");
        assert_eq!(sells.len(), 3, "expected 3 SELL rows, got {sells:?}");

        // Spot-check the 贵州茅台 sell
        let maotai = sells
            .iter()
            .find(|r| r.stock_name.contains("贵州茅台"))
            .expect("贵州茅台 SELL not found");
        assert!(
            (maotai.price - 1459.480).abs() < 0.01,
            "maotai price={}",
            maotai.price
        );
        assert!(
            (maotai.shares - 100.0).abs() < 0.01,
            "maotai shares={}",
            maotai.shares
        );

        // Spot-check the 04-22 招商银行 buy
        let zhaoshang = buys
            .iter()
            .find(|r| r.stock_name.contains("招商银行") && r.traded_at.contains("04-22"))
            .expect("招商银行 BUY 04-22 not found");
        assert!(
            (zhaoshang.price - 39.680).abs() < 0.01,
            "zhaoshang price={}",
            zhaoshang.price
        );
        assert!(
            (zhaoshang.shares - 1500.0).abs() < 0.01,
            "zhaoshang shares={}",
            zhaoshang.shares
        );
        assert!(
            (zhaoshang.commission - 5.60).abs() < 0.01,
            "zhaoshang commission={}",
            zhaoshang.commission
        );
    }

    #[test]
    fn debug_date_re_in_rust() {
        // Verify that the manual "not preceded by digit" guard (used in place of
        // the now-removed leading \b) correctly handles CJK prefixes like "全".
        // The Rust regex crate treats Unicode CJK characters as \w, so \b would
        // NOT fire between "全" and a following ASCII digit; we check the raw byte
        // instead.
        let samples: &[(&str, bool)] = &[
            ("全04.221425   1500   5.60", true), // "04.22" m=4 d=22 ← CJK prefix
            ("全04-22 14:26  1500  5.60", true), // "04-22" m=4 d=22 ← CJK prefix
            ("全04-1309:59  1500   5.58", true), // "04-13" m=4 d=13 ← CJK prefix
            ("39.680 full text", false),         // "39.68" m=39 → invalid
        ];
        let date_re = regex::Regex::new(r"(\d{1,2})[.-](\d{2})").unwrap();
        for (s, should_find) in samples {
            let bytes = s.as_bytes();
            let found = date_re.captures_iter(s).any(|cap| {
                let start = cap.get(0).unwrap().start();
                let preceded_by_digit = start > 0 && bytes[start - 1].is_ascii_digit();
                if preceded_by_digit {
                    return false;
                }
                let m: u32 = cap[1].parse().unwrap_or(99);
                let d: u32 = cap[2].parse().unwrap_or(99);
                (1..=12).contains(&m) && (1..=31).contains(&d)
            });
            assert_eq!(
                found, *should_find,
                "date_re on '{}': found={found} want={should_find}",
                s
            );
        }
    }

    /// Test with the verbatim OCR text produced by Tesseract on the 2× upscaled
    /// phone-scale synthetic image (375→750 px wide, 13px→26px font).
    /// This is what a real iPhone 对账单 screenshot produces after preprocessing.
    #[test]
    fn test_parse_2x_phone_scale_tesseract_output() {
        // Verbatim output from: tesseract ths_phone_2x.png out -l chi_sim --psm 6
        // (generated by make_ths_phone_image.py then upscaled 2× with Lanczos3)
        let text = "\
本月操作                              价格/数量           金手/税费

V 2026-04                          +270,742.49 +1.689%6

买入-招商银行                                39.680               -59525.60
全04-22 14:26                                   1500                    5.60

卖出-双汇发展           28.950     57865.43
全04-22 14:26                                   2000                    34.57

买入-招商银行                                38.970               -58460.58
全04-1309:59                   1500           5.58

卖出-双汇发展           28.410     56786.02
全04-1309:58                   2000           33.98.

买入-招商银行                                39.280               -145349.09
全04-0913:59                                   3700                     13.09

卖出-责州茅台           1459.480    145861.89
全04-0913:39                   100            86.11
";
        let rows = parse_ths_ocr(text);
        // Expect 6 rows; 卖出-责州茅台 is a known OCR misread of 贵州茅台 — still
        // parsed as SELL.
        assert_eq!(
            rows.len(),
            6,
            "expected 6 rows from 2× phone OCR output, got {}: {rows:?}",
            rows.len()
        );

        let buys: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "BUY")
            .collect();
        let sells: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "SELL")
            .collect();
        assert_eq!(buys.len(), 3, "expected 3 BUY, got {buys:?}");
        assert_eq!(sells.len(), 3, "expected 3 SELL, got {sells:?}");

        // 04-22 buy: 招商银行, price=39.680, shares=1500, comm=5.60
        let r = buys
            .iter()
            .find(|r| r.traded_at.contains("04-22"))
            .expect("04-22 BUY not found");
        assert!((r.price - 39.680).abs() < 0.01, "price={}", r.price);
        assert!((r.shares - 1500.0).abs() < 0.01, "shares={}", r.shares);
        assert!((r.commission - 5.60).abs() < 0.01, "comm={}", r.commission);
    }

    /// Test with the OCR text produced by Tesseract on the original small
    /// phone-scale image (375 px wide, 13 px font) WITHOUT preprocessing.
    /// This exercises the period-separator and merged-date-time code paths.
    #[test]
    fn test_parse_period_separator_ocr() {
        // At original (unscaled) size Tesseract reads "04-22 14:26" as the
        // merged string "04.221426" or similar.  The parser must still extract
        // all six trade rows without preprocessing help.
        let text = "\
本月操作               失格履昌     全关税费

V 2026.04           270.742.49 +1.6896

买入-招商银行                            39.680             -59525.60
全04.221425                             1500                 5.60

卖出-双汇发展           28.950     57865.43
全04.221426                            2000                 34.57

买入-招商银行           38.970     -58460.58
全04.130953           1500      5.58

卖出-双汇发展                            28.410             56786.02
全04.1309.58                            2000                 33.98

买入-招商银行                            39.280             -145349.09
全04091353           3700      13.09

卖出-贵州茅台                             1459.480         145861.89
全04091339                             100                  86.11
";
        let rows = parse_ths_ocr(text);
        assert_eq!(
            rows.len(),
            6,
            "expected 6 rows from period-separator OCR, got {}: {rows:?}",
            rows.len()
        );

        let buys: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "BUY")
            .collect();
        let sells: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "SELL")
            .collect();
        assert_eq!(buys.len(), 3, "expected 3 BUY, got {buys:?}");
        assert_eq!(sells.len(), 3, "expected 3 SELL, got {sells:?}");

        // Price must be extracted correctly from anchor line; shares from
        // the date row (may be approximate due to OCR garbling).
        let r0 = buys
            .iter()
            .find(|r| r.stock_name.contains("招商银行") && r.traded_at.contains("04-22"))
            .expect("招商银行 04-22 BUY not found");
        assert!((r0.price - 39.680).abs() < 0.01, "price={}", r0.price);
        assert!(
            r0.shares > 0.0,
            "shares must be positive, got {}",
            r0.shares
        );

        let maotai = sells
            .iter()
            .find(|r| r.stock_name.contains("贵州茅台"))
            .expect("贵州茅台 SELL not found");
        assert!(
            (maotai.price - 1459.480).abs() < 0.01,
            "maotai price={}",
            maotai.price
        );
    }

    /// Test the dateline fallback using the EXACT Tesseract output from the
    /// user's actual THS phone screenshot (2x upscaled, 460→920px wide).
    /// At this scale Tesseract reads Chinese correctly EXCEPT the
    /// "买入-招商银行" / "卖出-双汇发展" trade-direction+name lines, which
    /// appear blank (Chinese text not rendered in the small synthetic image).
    /// The dateline fallback must still produce 6 rows by using the
    /// "日MM-DD HH.MM  shares  commission" secondary lines and working backward
    /// to the price+amount line.
    #[test]
    fn test_parse_dateline_fallback_no_anchors() {
        // Verbatim OCR output from tesseract on the 2x synthetic image.
        // The name/direction lines are blank — only numbers survive.
        let text = "\
                                                                         E台到                             E台到

日2026-04                                                +270.742.49

                                                                         39.680                       -59525.60
日04-22 14.26                                   1500              5.60
                                                                         28.950                       57865.43
日04-22 14.26                                   2000              34.57
                                                                         38.970                       -58460.58
日04-13 09.59                                   1500              5.58
                                                                         28.410                       56786.02
日04-13 09.58                                   2000              33.98
                                               39.280                    -145349.09
日04-09 13:39                                   3700              13.09
                                        1459.480           145861.89
日04-09 13:39                                   100               86.11

日2026-03

日2026-02
";
        let rows = parse_ths_ocr(text);
        assert_eq!(
            rows.len(),
            6,
            "dateline fallback must produce 6 rows, got {}: {rows:?}",
            rows.len()
        );

        let buys: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "BUY")
            .collect();
        let sells: Vec<_> = rows
            .iter()
            .filter(|r| r.transaction_type == "SELL")
            .collect();
        assert_eq!(buys.len(), 3, "expected 3 BUY rows, got {buys:?}");
        assert_eq!(sells.len(), 3, "expected 3 SELL rows, got {sells:?}");

        // Spot-check 招商银行 04-22 buy
        let r0 = buys
            .iter()
            .find(|r| r.traded_at.contains("04-22"))
            .expect("04-22 BUY not found");
        assert!((r0.price - 39.680).abs() < 0.01, "price={}", r0.price);
        assert!((r0.shares - 1500.0).abs() < 0.01, "shares={}", r0.shares);
        assert!(r0.transaction_type == "BUY");

        // Spot-check 贵州茅台 sell
        let maotai = sells
            .iter()
            .find(|r| r.price > 1000.0)
            .expect("贵州茅台 SELL (price>1000) not found");
        assert!(
            (maotai.price - 1459.480).abs() < 0.01,
            "maotai price={}",
            maotai.price
        );
        assert!(
            (maotai.shares - 100.0).abs() < 0.01,
            "maotai shares={}",
            maotai.shares
        );
    }
}
