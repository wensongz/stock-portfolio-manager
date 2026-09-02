use super::*;

// ---------------------------------------------------------------------------
// Image slicing — split THS screenshot into individual trade-card images
// ---------------------------------------------------------------------------

/// Minimum pixel height for a valid trade-card slice.  Slices shorter than
/// this are discarded as separator artefacts or padding-only strips.
const MIN_CARD_HEIGHT_PX: u32 = 30;

/// Minimum height (in pixels) that a separator band must span before it is
/// treated as a genuine inter-card boundary.
///
/// In high-resolution phone screenshots (e.g., 3× Retina, ~1170 px wide)
/// each UI row is 88–132 px tall, leaving 24–46 px of blank padding above
/// and below the text.  Those padding rows are also detected as "separator
/// candidates" (high mean luminance, low range).  We must ignore them.
///
/// A genuine between-card separator (e.g., the light-gray group-header
/// background in the ∧ 2026-04 row) spans at least one full row (~88 px)
/// of uniform light color, whereas in-row padding is typically < 50 px.
///
/// Setting the threshold to 50 px keeps us from splitting a single 2-line
/// trade entry in half while still allowing detection of obvious card
/// separators in lower-resolution "card" layouts.
const MIN_SEPARATOR_BAND_PX: u32 = 50;

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
pub(super) fn split_image_by_separators(data: &[u8]) -> Vec<Vec<u8>> {
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
            if lum < min_lum {
                min_lum = lum;
            }
            if lum > max_lum {
                max_lum = lum;
            }
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
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
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
// Image pre-processing
// ---------------------------------------------------------------------------

/// Pre-process raw image bytes to improve Tesseract OCR accuracy on mobile
/// phone screenshots:
///
/// 1. **Upscale**: if the image is narrower than `MIN_OCR_WIDTH` pixels,
///    scale it up so that CJK characters are large enough for chi_sim
///    (Tesseract accuracy degrades sharply when characters are < 20 px tall;
///    a typical 375 px-wide phone screenshot has ~13 px text).
///
/// 2. **Grayscale + binarize**: convert to luma and threshold at
///    `OCR_THRESHOLD`.  Any pixel with luminance < threshold becomes black
///    (text) and anything ≥ threshold becomes white (background).
///    This collapses coloured trade text (red negative amounts, blue dates)
///    into plain black, removes JPEG compression noise, and gives Tesseract
///    a clean high-contrast input.
///
/// 3. Re-encode as PNG (lossless) before handing off to Tesseract.
///
/// Falls back to the original bytes on any image-decoding error.
pub(super) fn preprocess_for_ocr(data: &[u8]) -> Vec<u8> {
    use image::GenericImageView as _;
    let img = match image::load_from_memory(data) {
        Ok(i) => i,
        Err(_) => return data.to_vec(),
    };

    // ── 1. Upscale if too small ───────────────────────────────────────────────
    const MIN_OCR_WIDTH: u32 = 1000;
    let (w, h) = img.dimensions();
    let img = if w < MIN_OCR_WIDTH {
        // Ceiling: scale up so result width ≥ MIN_OCR_WIDTH.
        let scale = MIN_OCR_WIDTH.div_ceil(w);
        img.resize(
            w * scale,
            h * scale,
            image::imageops::FilterType::CatmullRom,
        )
    } else {
        img
    };

    // ── 2. Grayscale ──────────────────────────────────────────────────────────
    let gray = img.to_luma8();

    // ── 3. Binarize ───────────────────────────────────────────────────────────
    // Threshold 180 keeps all coloured text (red amounts ≈ luma 118, blue
    // dates ≈ luma 76) as black while turning white/light-grey background
    // pure white.
    const OCR_THRESHOLD: u8 = 180;
    let binary = image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_fn(
        gray.width(),
        gray.height(),
        |x, y| {
            let l = gray.get_pixel(x, y)[0];
            image::Luma([if l < OCR_THRESHOLD { 0u8 } else { 255u8 }])
        },
    );

    let mut buf: Vec<u8> = Vec::new();
    if image::DynamicImage::ImageLuma8(binary)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .is_ok()
    {
        buf
    } else {
        data.to_vec()
    }
}

// ---------------------------------------------------------------------------
// OCR helper
// ---------------------------------------------------------------------------

///
/// `data` is the raw PNG/JPEG file content.  Tesseract is invoked via
/// `std::process::Command` so no native library linking is required.
pub(super) fn ocr_image(data: &[u8]) -> Result<String, String> {
    // Pre-process: upscale small images and binarize to improve chi_sim accuracy
    // on mobile phone screenshots.  On any processing failure, fall back to the
    // original bytes so OCR can still be attempted.
    let processed = preprocess_for_ocr(data);

    // Write image bytes to a temp file.
    let mut tmp = tempfile::Builder::new()
        .suffix(".png")
        .tempfile()
        .map_err(|e| format!("创建临时文件失败: {}", e))?;
    tmp.write_all(&processed)
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
    std::fs::read_to_string(&out_file).map_err(|e| format!("读取 OCR 结果失败: {}", e))
}
