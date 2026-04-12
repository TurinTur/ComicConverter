use image::{DynamicImage, GenericImageView, Pixel, Rgba};
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
        let mut img = image::open(source_path)
            .map_err(|e| format!("Failed to open image {:?}: {}", source_path, e))?;

        if trim_pages {
            // Equivalent to standard Trim, but without native full fuzz trim in the standard `image` crate easily
            // We use standard Magick's trim behavior port. For simplicity, we can do a smart trim 
            // with 100% threshold and small fuzz for standard trim as well.
            if let Some(bounds) = Self::calculate_smart_trim_bounds(&img, 1.0, 5.0) {
                if (bounds.width() as f64) >= (img.width() as f64) * trim_min_size
                    && (bounds.height() as f64) >= (img.height() as f64) * trim_min_size
                {
                    img = img.crop_imm(bounds.0, bounds.1, bounds.2, bounds.3);
                }
            }
        }

        if smart_trim_pages {
            if let Some(bounds) = Self::calculate_smart_trim_bounds(&img, smart_trim_threshold, smart_trim_tolerance) {
                if (bounds.width() as f64) >= (img.width() as f64) * trim_min_size
                    && (bounds.height() as f64) >= (img.height() as f64) * trim_min_size
                {
                    img = img.crop_imm(bounds.0, bounds.1, bounds.2, bounds.3);
                }
            }
        }

        if let Some(pct) = resize_percentage {
            if (pct - 100.0).abs() > f32::EPSILON {
                let nwidth = ((img.width() as f32) * (pct / 100.0)).round() as u32;
                let nheight = ((img.height() as f32) * (pct / 100.0)).round() as u32;
                img = img.resize(nwidth, nheight, image::imageops::FilterType::Lanczos3);
            }
        }

        let encoder = Encoder::from_image(&img)
            .map_err(|e| format!("WebP Encode Error: {:?}", e))?;
        let webp_memory = encoder.encode(quality); // webp 0.3 returns WebPMemory

        std::fs::write(dest_path, &*webp_memory)
            .map_err(|e| format!("Failed to write {:?}: {}", dest_path, e))?;

        Ok(())
    }

    /// Port of CalculateSmartTrimBounds from C#
    fn calculate_smart_trim_bounds(
        img: &DynamicImage,
        row_bg_threshold: f64,
        color_tolerance_pct: f64,
    ) -> Option<Rect> {
        let (width, height) = img.dimensions();
        if width <= 1 || height <= 1 {
            return None;
        }

        // Average 4 corners
        let corners = [(0, 0), (width - 1, 0), (0, height - 1), (width - 1, height - 1)];
        let mut bg_r = 0.0;
        let mut bg_g = 0.0;
        let mut bg_b = 0.0;
        let mut corner_count = 0;

        for &(cx, cy) in &corners {
            let rgba = img.get_pixel(cx, cy);
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

        let tolerance = 255.0 * (color_tolerance_pct / 100.0);

        let is_background = |x: u32, y: u32| -> bool {
            let p = img.get_pixel(x, y);
            let diff = ((p[0] as f64 - bg_r).abs() +
                       (p[1] as f64 - bg_g).abs() +
                       (p[2] as f64 - bg_b).abs()) / 3.0;
            diff <= tolerance
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
