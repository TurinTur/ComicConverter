use image::RgbaImage;
use webp::Encoder;
use std::path::Path;

pub struct ImageEngine;

impl ImageEngine {
    pub fn process_image(
        source_path: &Path,
        dest_path: &Path,
        trim_pages: bool,
        smart_trim_pages: bool,
        trim_min_size: f64,
        smart_trim_threshold: f64,
        smart_trim_tolerance: f64,
        resize_percentage: Option<f32>,
        quality: f32,
    ) -> Result<(), String> {
        // PERFORMANCE: convert to RGBA exactly once and stay in RgbaImage land for
        // the whole pipeline (trim -> resize -> encode). Previously every trim pass
        // re-converted the whole image to RGBA again.
        let img = image::open(source_path)
            .map_err(|e| format!("Failed to open image {:?}: {}", source_path, e))?;
        let mut cur: RgbaImage = img.to_rgba8();

        if trim_pages {
            // Equivalent to standard Trim, but without native full fuzz trim in the standard `image` crate easily
            // We use standard Magick's trim behavior port. For simplicity, we can do a smart trim 
            // with 100% threshold and small fuzz for standard trim as well.
            if let Some(bounds) = Self::calculate_smart_trim_bounds(&cur, 1.0, 5.0) {
                if (bounds.width() as f64) >= (cur.width() as f64) * trim_min_size
                    && (bounds.height() as f64) >= (cur.height() as f64) * trim_min_size
                {
                    cur = image::imageops::crop_imm(&cur, bounds.0, bounds.1, bounds.2, bounds.3).to_image();
                }
            }
        }

        if smart_trim_pages {
            if let Some(bounds) = Self::calculate_smart_trim_bounds(&cur, smart_trim_threshold, smart_trim_tolerance) {
                if (bounds.width() as f64) >= (cur.width() as f64) * trim_min_size
                    && (bounds.height() as f64) >= (cur.height() as f64) * trim_min_size
                {
                    cur = image::imageops::crop_imm(&cur, bounds.0, bounds.1, bounds.2, bounds.3).to_image();
                }
            }
        }

        if let Some(pct) = resize_percentage {
            if (pct - 100.0).abs() > f32::EPSILON {
                let nwidth = ((cur.width() as f32) * (pct / 100.0)).round().max(1.0) as u32;
                let nheight = ((cur.height() as f32) * (pct / 100.0)).round().max(1.0) as u32;
                cur = image::imageops::resize(&cur, nwidth, nheight, image::imageops::FilterType::Lanczos3);
            }
        }

        // Encode straight from the RGBA buffer (no extra conversion inside the encoder).
        let (w, h) = cur.dimensions();
        let encoder = Encoder::from_rgba(cur.as_raw(), w, h);
        let webp_memory = encoder.encode(quality);

        std::fs::write(dest_path, &*webp_memory)
            .map_err(|e| format!("Failed to write {:?}: {}", dest_path, e))?;

        Ok(())
    }

    /// Port of CalculateSmartTrimBounds from C#
    /// PERFORMANCE: takes the raw RGBA buffer directly (converted once upstream)
    /// and uses integer math for the per-pixel background check.
    fn calculate_smart_trim_bounds(
        img: &RgbaImage,
        row_bg_threshold: f64,
        color_tolerance_pct: f64,
    ) -> Option<Rect> {
        let (width, height) = img.dimensions();
        if width <= 1 || height <= 1 {
            return None;
        }

        let buf = img.as_raw();
        let px = |x: u32, y: u32| -> [u8; 4] {
            let idx = ((y as usize * width as usize) + x as usize) * 4;
            [buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3]]
        };

        // Average 4 corners
        let corners = [(0, 0), (width - 1, 0), (0, height - 1), (width - 1, height - 1)];
        let mut bg_r = 0.0;
        let mut bg_g = 0.0;
        let mut bg_b = 0.0;
        let mut corner_count = 0;

        for &(cx, cy) in &corners {
            let rgba = px(cx, cy);
            bg_r += rgba[0] as f64;
            bg_g += rgba[1] as f64;
            bg_b += rgba[2] as f64;
            corner_count += 1;
        }

        if corner_count == 0 {
            return None;
        }

        bg_r /= corner_count as f64;
        bg_g /= corner_count as f64;
        bg_b /= corner_count as f64;

        // PERFORMANCE: integer math for the per-pixel check (|a-b| via saturating
        // abs_diff) instead of three f64 conversions + float ops per pixel.
        let tolerance_i = (255.0 * (color_tolerance_pct / 100.0)).round() as i32;

        let is_background = |x: u32, y: u32| -> bool {
            let p = px(x, y);
            let diff = (p[0].abs_diff(bg_r as u8) as i32
                + p[1].abs_diff(bg_g as u8) as i32
                + p[2].abs_diff(bg_b as u8) as i32)
                / 3;
            diff <= tolerance_i
        };

        let row_bg_fraction = |y: u32, x_from: u32, x_to: u32| -> f64 {
            let total = x_to.saturating_sub(x_from) + 1;
            let mut count = 0;
            for x in x_from..=x_to {
                if is_background(x, y) {
                    count += 1;
                }
            }
            if total > 0 {
                (count as f64) / (total as f64)
            } else {
                1.0
            }
        };

        let col_bg_fraction = |x: u32, y_from: u32, y_to: u32| -> f64 {
            let total = y_to.saturating_sub(y_from) + 1;
            let mut count = 0;
            for y in y_from..=y_to {
                if is_background(x, y) {
                    count += 1;
                }
            }
            if total > 0 {
                (count as f64) / (total as f64)
            } else {
                1.0
            }
        };

        let mut top = 0;
        for y in 0..(height / 2) {
            if row_bg_fraction(y, 0, width - 1) >= row_bg_threshold {
                top = y + 1;
            } else {
                break;
            }
        }

        let mut bottom = height - 1;
        for y in (top..=(height - 1)).rev() {
            if row_bg_fraction(y, 0, width - 1) >= row_bg_threshold {
                bottom = y.saturating_sub(1);
            } else {
                break;
            }
        }

        let mut left = 0;
        for x in 0..(width / 2) {
            if col_bg_fraction(x, top, bottom) >= row_bg_threshold {
                left = x + 1;
            } else {
                break;
            }
        }

        let mut right = width - 1;
        for x in (left..=(width - 1)).rev() {
            if col_bg_fraction(x, top, bottom) >= row_bg_threshold {
                right = x.saturating_sub(1);
            } else {
                break;
            }
        }

        if left >= right || top >= bottom {
            return None;
        }

        if left == 0 && top == 0 && right == width - 1 && bottom == height - 1 {
            return None;
        }

        Some(Rect(left, top, right - left + 1, bottom - top + 1))
    }
}

pub struct Rect(pub u32, pub u32, pub u32, pub u32);

impl Rect {
    pub fn width(&self) -> u32 { self.2 }
    pub fn height(&self) -> u32 { self.3 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn trims_uniform_border() {
        let mut img = solid(100, 100, [255, 255, 255, 255]);
        for y in 10..=89 {
            for x in 20..=79 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let r = ImageEngine::calculate_smart_trim_bounds(&img, 1.0, 5.0).unwrap();
        assert_eq!((r.0, r.1, r.width(), r.height()), (20, 10, 60, 80));
    }

    #[test]
    fn fully_background_returns_none() {
        let img = solid(50, 50, [255, 255, 255, 255]);
        assert!(ImageEngine::calculate_smart_trim_bounds(&img, 1.0, 5.0).is_none());
    }

    #[test]
    fn tiny_image_returns_none() {
        assert!(ImageEngine::calculate_smart_trim_bounds(&solid(1, 1, [0, 0, 0, 255]), 1.0, 5.0).is_none());
        assert!(ImageEngine::calculate_smart_trim_bounds(&solid(2, 1, [0, 0, 0, 255]), 1.0, 5.0).is_none());
        assert!(ImageEngine::calculate_smart_trim_bounds(&solid(1, 2, [0, 0, 0, 255]), 1.0, 5.0).is_none());
    }
}
