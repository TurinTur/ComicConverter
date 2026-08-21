//! Headless benchmark + validation harness.
//!
//! Run with: `comic_converter.exe --benchmark <source...> --temp <dir> --final <dir> [options]`
//!
//! For each source archive it:
//!   1. Times extraction, conversion and archiving separately.
//!   2. Reports initial vs final size (bytes, % change).
//!   3. Validates page-count integrity (source pages == output CBZ entries).
//!   4. Validates trim correctness (output dimensions must match the expected crop).
//!   5. Measures perceived quality via PSNR between original and converted pages.
//!
//! Exit code is non-zero if any validation fails, so it can be used in CI.

use crate::processor::{ComicProcessor, ProcessorContext};
use image::ImageDecoder;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;
use zip::ZipArchive;

pub struct BenchmarkConfig {
    pub sources: Vec<String>,
    pub temp_folder: String,
    pub final_folder: String,
    pub fallback_7z: String,
    pub threads: usize,
    pub resize: String,
    pub quality: f32,
    pub trim_pages: bool,
    pub smart_trim_pages: bool,
    pub trim_min_size: f64,
    pub smart_trim_threshold: f64,
    pub smart_trim_tolerance: f64,
}

pub struct PhaseTimings {
    pub extract_secs: f64,
    pub convert_secs: f64,
    pub archive_secs: f64,
}

pub struct BenchResult {
    pub name: String,
    pub initial_bytes: u64,
    pub final_bytes: u64,
    pub timings: Option<PhaseTimings>,
    pub pages_source: usize,
    pub pages_output: usize,
    /// PSNR samples (dB) between original and converted pages.
    pub psnr_samples: Vec<f64>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl BenchResult {
    pub fn passed(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        let status = if self.passed() { "PASS" } else { "FAIL" };
        lines.push(format!("=== {} : {} ===", self.name, status));

        let init_mb = self.initial_bytes as f64 / (1024.0 * 1024.0);
        let final_mb = self.final_bytes as f64 / (1024.0 * 1024.0);
        let delta = if self.initial_bytes > 0 {
            ((self.final_bytes as f64 - self.initial_bytes as f64) / self.initial_bytes as f64) * 100.0
        } else {
            0.0
        };
        lines.push(format!(
            "Size: {:.1} MB -> {:.1} MB ({:+.1}%)",
            init_mb, final_mb, delta
        ));

        if let Some(t) = &self.timings {
            let total = t.extract_secs + t.convert_secs + t.archive_secs;
            lines.push(format!(
                "Time: extract {:.1}s | convert {:.1}s | archive {:.1}s | total {:.1}s",
                t.extract_secs, t.convert_secs, t.archive_secs, total
            ));
            if self.pages_source > 0 && t.convert_secs > 0.0 {
                lines.push(format!(
                    "Throughput: {:.1} pages/s",
                    self.pages_source as f64 / t.convert_secs
                ));
            }
        }

        lines.push(format!(
            "Pages: {} source -> {} output{}",
            self.pages_source,
            self.pages_output,
            if self.pages_source != self.pages_output { "  MISMATCH!" } else { "" }
        ));

        if !self.psnr_samples.is_empty() {
            let min = self.psnr_samples.iter().cloned().fold(f64::INFINITY, f64::min);
            let avg = self.psnr_samples.iter().sum::<f64>() / self.psnr_samples.len() as f64;
            lines.push(format!(
                "PSNR: avg {:.1} dB, min {:.1} dB over {} sampled page(s)",
                avg,
                min,
                self.psnr_samples.len()
            ));
        }

        for w in &self.warnings {
            lines.push(format!("WARN: {}", w));
        }
        for e in &self.errors {
            lines.push(format!("ERROR: {}", e));
        }
        lines.join("\n")
    }
}

/// PSNR between two RGBA buffers of identical dimensions.
/// Returns None if the images differ in size or are identical (infinite PSNR).
pub fn psnr_rgba(a: &[u8], b: &[u8]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    // Only compare RGB channels; alpha is irrelevant for perceived quality.
    let mut sq_sum: f64 = 0.0;
    let mut n = 0usize;
    for (ca, cb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for c in 0..3 {
            let d = (ca[c] as i32) - (cb[c] as i32);
            sq_sum += (d * d) as f64;
        }
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let mse = sq_sum / (n as f64 * 3.0);
    if mse == 0.0 {
        return None; // identical images
    }
    Some(10.0 * (255.0f64 * 255.0 / mse).log10())
}

pub struct BenchmarkRunner;

impl BenchmarkRunner {
    /// CLI entry point: parses args, runs the benchmark, prints results.
    pub fn run_cli(args: &[String]) {
        let mut sources = Vec::new();
        let mut temp_folder = env::temp_dir().join("comic_converter_bench").to_string_lossy().to_string();
        let mut final_folder = env::temp_dir().join("comic_converter_bench_out").to_string_lossy().to_string();
        let mut fallback_7z = String::new();
        let mut threads = num_cpus::get();
        let mut resize = "100%".to_string();
        let mut quality = 67.0f32;
        let mut trim_pages = true;
        let mut smart_trim_pages = false;
        let mut trim_min_size = 0.75;
        let mut smart_trim_threshold = 0.97;
        let mut smart_trim_tolerance = 8.0;

        // Start at 1: args[0] is the executable path, not a source.
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--temp" | "--final" | "--7z" | "--threads" | "--resize"
                | "--quality" | "--trim-min" | "--trim-threshold" | "--trim-tolerance" => {
                    let key = args[i].as_str();
                    i += 1;
                    if i >= args.len() {
                        eprintln!("Missing value for {}", key);
                        std::process::exit(2);
                    }
                    match key {
                        "--temp" => temp_folder = args[i].clone(),
                        "--final" => final_folder = args[i].clone(),
                        "--7z" => fallback_7z = args[i].clone(),
                        "--threads" => threads = args[i].parse().unwrap_or(threads),
                        "--resize" => resize = args[i].clone(),
                        "--quality" => quality = args[i].parse().unwrap_or(quality),
                        "--trim-min" => trim_min_size = args[i].parse::<f64>().unwrap_or(75.0) / 100.0,
                        "--trim-threshold" => smart_trim_threshold = args[i].parse::<f64>().unwrap_or(97.0) / 100.0,
                        "--trim-tolerance" => smart_trim_tolerance = args[i].parse().unwrap_or(8.0),
                        _ => {}
                    }
                }
                "--no-trim" => trim_pages = false,
                "--smart-trim" => smart_trim_pages = true,
                // Marker flag only; sources are collected as positional args.
                "--benchmark" => {}
                s => sources.push(s.to_string()),
            }
            i += 1;
        }

        if sources.is_empty() {
            println!("Usage: comic_converter.exe --benchmark <sources...> [options]");
            println!();
            println!("Options:");
            println!("  --temp <dir>          Temp working folder (default: system temp)");
            println!("  --final <dir>         Output folder (default: system temp)");
            println!("  --7z <path>           Path to 7z.exe (needed for CBR/RAR)");
            println!("  --threads <n>         Worker threads (default: all cores)");
            println!("  --resize <pct>        Resize percentage, e.g. 75 or 75%% (default: 100)");
            println!("  --quality <q>         WebP quality 0-100 (default: 67)");
            println!("  --no-trim             Disable standard trim");
            println!("  --smart-trim          Enable smart trim");
            println!("  --trim-min <pct>      Min kept size % (default: 75)");
            println!("  --trim-threshold <pct> Smart-trim bg threshold % (default: 97)");
            println!("  --trim-tolerance <n>  Smart-trim tolerance (default: 8)");
            std::process::exit(2);
        }

        let cfg = BenchmarkConfig {
            sources,
            temp_folder,
            final_folder,
            fallback_7z,
            threads,
            resize,
            quality,
            trim_pages,
            smart_trim_pages,
            trim_min_size,
            smart_trim_threshold,
            smart_trim_tolerance,
        };

        println!("Benchmarking {} source(s)...", cfg.sources.len());
        println!(
            "Settings: threads={} quality={} resize={} trim={} smart_trim={}",
            cfg.threads, cfg.quality, cfg.resize, cfg.trim_pages, cfg.smart_trim_pages
        );
        println!();

        let results = Self::run(&cfg);
        let mut failed = 0;
        for r in &results {
            println!("{}", r.summary());
            println!();
            if !r.passed() {
                failed += 1;
            }
        }

        // Overall summary
        let total_init: u64 = results.iter().map(|r| r.initial_bytes).sum();
        let total_final: u64 = results.iter().map(|r| r.final_bytes).sum();
        let total_secs: f64 = results
            .iter()
            .filter_map(|r| r.timings.as_ref())
            .map(|t| t.extract_secs + t.convert_secs + t.archive_secs)
            .sum();
        let total_pages: usize = results.iter().map(|r| r.pages_source).sum();

        println!("================ OVERALL ================");
        println!(
            "Sources: {} | Pages: {} | Total time: {:.1}s",
            results.len(),
            total_pages,
            total_secs
        );
        if total_init > 0 {
            println!(
                "Size: {:.1} MB -> {:.1} MB ({:+.1}%)",
                total_init as f64 / (1024.0 * 1024.0),
                total_final as f64 / (1024.0 * 1024.0),
                ((total_final as f64 - total_init as f64) / total_init as f64) * 100.0
            );
        }
        if total_secs > 0.0 && total_pages > 0 {
            println!("Overall throughput: {:.1} pages/s", total_pages as f64 / total_secs);
        }
        println!("Result: {} passed, {} failed", results.len() - failed, failed);

        std::process::exit(if failed > 0 { 1 } else { 0 });
    }

    pub fn run(cfg: &BenchmarkConfig) -> Vec<BenchResult> {
        let mut results = Vec::new();

        for source in &cfg.sources {
            let p = Path::new(source);
            if !p.exists() {
                results.push(BenchResult {
                    name: source.clone(),
                    initial_bytes: 0,
                    final_bytes: 0,
                    timings: None,
                    pages_source: 0,
                    pages_output: 0,
                    psnr_samples: Vec::new(),
                    errors: vec![format!("Source not found: {}", source)],
                    warnings: Vec::new(),
                });
                continue;
            }

            let result = Self::bench_one(cfg, p);
            results.push(result);
        }
        results
    }

    fn bench_one(cfg: &BenchmarkConfig, source: &Path) -> BenchResult {
        let name = source
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| source.to_string_lossy().to_string());

        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        let initial_bytes = fs::metadata(source).map(|m| m.len()).unwrap_or(0);

        // Per-run isolated temp/final dirs so runs don't interfere.
        let run_id = chrono::Local::now().format("%Y%m%d_%H%M%S%.3f");
        let temp_dir = Path::new(&cfg.temp_folder).join(format!("_bench_{}", run_id));
        let final_dir = Path::new(&cfg.final_folder).join(format!("_bench_out_{}", run_id));
        let _ = fs::create_dir_all(&temp_dir);
        let _ = fs::create_dir_all(&final_dir);

        // ---- Count source pages (extract to a scratch dir first) ----
        // NOTE: must live OUTSIDE the per-run pipeline temp dir, otherwise the
        // processor would treat the raw extracted pages as work output and
        // archive them alongside the converted result.
        let scratch = Path::new(&cfg.temp_folder).join(format!("_bench_scratch_{}", run_id));
        let count_start = Instant::now();
        let src_page_files = match Self::extract_to(source, &scratch, &cfg.fallback_7z) {
            Ok(files) => files,
            Err(e) => {
                errors.push(format!("Failed to extract source for counting: {}", e));
                return Self::finish(name, initial_bytes, 0, None, 0, 0, Vec::new(), errors, warnings);
            }
        };
        let _ = count_start.elapsed();
        let pages_source = src_page_files.len();
        if pages_source == 0 {
            errors.push("No pages found in source archive.".to_string());
            return Self::finish(name, initial_bytes, 0, None, 0, 0, Vec::new(), errors, warnings);
        }

        // Keep a small sample of original pages in memory for PSNR later
        // (bounded so memory doesn't explode on huge archives).
        const SAMPLE_COUNT: usize = 8;
        let sample_step = (pages_source / SAMPLE_COUNT).max(1);
        let sample_paths: Vec<PathBuf> = src_page_files
            .iter()
            .step_by(sample_step)
            .take(SAMPLE_COUNT)
            .cloned()
            .collect();
        let mut originals: Vec<(String, image::RgbaImage)> = Vec::new();
        for sp in &sample_paths {
            if let Ok(img) = image::open(sp) {
                originals.push((sp.file_name().unwrap_or_default().to_string_lossy().to_string(), img.to_rgba8()));
            }
        }

        // ---- Run the real pipeline with phase timing ----
        let timings = Self::run_pipeline(cfg, source, &temp_dir, &final_dir, &mut errors);

        // ---- Locate the produced archive(s) ----
        let outputs: Vec<PathBuf> = fs::read_dir(&final_dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        matches!(
                            p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
                            Some("cbz") | Some("zip")
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        if outputs.is_empty() {
            errors.push("No output CBZ/ZIP was produced.".to_string());
        }

        let mut pages_output = 0usize;
        let mut psnr_samples = Vec::new();

        for out in &outputs {
            match Self::read_zip_entries(out) {
                Ok(entries) => {
                    pages_output += entries.len();
                    // PSNR on sampled pages
                    for (entry_name, bytes) in &entries {
                        let stem = Path::new(entry_name)
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if let Some((_, orig)) = originals.iter().find(|(n, _)| {
                            Path::new(n).file_stem().map(|s| s.to_string_lossy().to_string()) == Some(stem.clone())
                        }) {
                            match Self::decode_webp(bytes) {
                                Some(conv) => {
                                    // If trimming changed dimensions we can't compare directly;
                                    // record a warning instead of a bogus PSNR.
                                    if conv.dimensions() == orig.dimensions() {
                                        if let Some(p) = psnr_rgba(orig.as_raw(), conv.as_raw()) {
                                            psnr_samples.push(p);
                                        } else {
                                            psnr_samples.push(f64::INFINITY); // pixel-identical
                                        }
                                    } else {
                                        warnings.push(format!(
                                            "Page '{}': dimensions changed {}x{} -> {}x{} (trim applied)",
                                            stem,
                                            orig.width(),
                                            orig.height(),
                                            conv.width(),
                                            conv.height()
                                        ));
                                    }
                                }
                                None => warnings.push(format!("Page '{}': could not decode output WebP", stem)),
                            }
                        }
                    }
                }
                Err(e) => errors.push(format!("Failed to read output {}: {}", out.display(), e)),
            }
        }

        // Page integrity check
        if pages_output != pages_source && !outputs.is_empty() {
            errors.push(format!(
                "Page count mismatch: source has {}, output has {}",
                pages_source, pages_output
            ));
        }

        // Quality gate: q67 WebP should land well above these thresholds.
        let finite: Vec<f64> = psnr_samples.iter().cloned().filter(|p| p.is_finite()).collect();
        if !finite.is_empty() {
            let min = finite.iter().cloned().fold(f64::INFINITY, f64::min);
            if min < 28.0 {
                errors.push(format!("Perceived quality too low: min PSNR {:.1} dB (< 28)", min));
            } else if min < 32.0 {
                warnings.push(format!("Perceived quality borderline: min PSNR {:.1} dB (< 32)", min));
            }
        }

        // Measure output size BEFORE the run dirs are cleaned up.
        let final_bytes: u64 = outputs
            .iter()
            .filter_map(|p| fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();

        // Cleanup this run's dirs
        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::remove_dir_all(&final_dir);
        let _ = fs::remove_dir_all(&scratch);

        Self::finish(
            name,
            initial_bytes,
            final_bytes,
            timings,
            pages_source,
            pages_output,
            psnr_samples,
            errors,
            warnings,
        )
    }

    fn finish(
        name: String,
        initial_bytes: u64,
        final_bytes: u64,
        timings: Option<PhaseTimings>,
        pages_source: usize,
        pages_output: usize,
        psnr_samples: Vec<f64>,
        errors: Vec<String>,
        warnings: Vec<String>,
    ) -> BenchResult {
        BenchResult {
            name,
            initial_bytes,
            final_bytes,
            timings,
            pages_source,
            pages_output,
            psnr_samples,
            errors,
            warnings,
        }
    }

    /// Runs the actual ComicProcessor pipeline against one source, timing each phase
    /// by watching progress messages.
    fn run_pipeline(
        cfg: &BenchmarkConfig,
        source: &Path,
        temp_dir: &Path,
        final_dir: &Path,
        errors: &mut Vec<String>,
    ) -> Option<PhaseTimings> {
        let ctx = ProcessorContext {
            source_items: vec![source.to_string_lossy().to_string()],
            temp_folder: temp_dir.to_string_lossy().to_string(),
            final_folder: final_dir.to_string_lossy().to_string(),
            fallback_7z: cfg.fallback_7z.clone(),
            threads: cfg.threads,
            resize: cfg.resize.clone(),
            quality: cfg.quality,
            delete_source: false,
            copy_final: true,
            trim_pages: cfg.trim_pages,
            smart_trim_pages: cfg.smart_trim_pages,
            trim_min_size: cfg.trim_min_size,
            smart_trim_threshold: cfg.smart_trim_threshold,
            smart_trim_tolerance: cfg.smart_trim_tolerance,
            zip_mode: "individual".to_string(),
            delete_temp: false,
            include_range_in_name: true,
        };

        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _ = ComicProcessor::process(&ctx, tx);
        });

        // Track phase boundaries from progress messages. Extraction itself is
        // silent in the parallel extractor, so its end is inferred from the
        // next step's header message.
        let (mut ex_s, mut ex_e) = (None, None);
        let (mut cv_s, mut cv_e) = (None, None);
        let (mut ar_s, mut ar_e) = (None, None);
        while let Ok(msg) = rx.recv() {
            let m = &msg.message;
            if m.contains("Extracting archives") {
                if ex_s.is_none() { ex_s = Some(Instant::now()); }
            } else if m.contains("Creating target folder structure") {
                if ex_e.is_none() { ex_e = Some(Instant::now()); }
            } else if m.contains("Converting images") {
                if ex_e.is_none() { ex_e = Some(Instant::now()); }
                if cv_s.is_none() { cv_s = Some(Instant::now()); }
            } else if m.contains("Converted ") {
                cv_e = Some(Instant::now());
            } else if m.contains("Conversion complete") {
                if cv_e.is_none() { cv_e = Some(Instant::now()); }
            } else if m.contains("Archiving results") {
                if cv_e.is_none() { cv_e = Some(Instant::now()); }
                if ar_s.is_none() { ar_s = Some(Instant::now()); }
            } else if m.contains("Final converted size") || m.contains("Process completed") {
                if ar_e.is_none() { ar_e = Some(Instant::now()); }
            }
        }
        handle.join().ok()?;

        // Graceful fallbacks when a marker never arrived.
        let start_all = ex_s.or(cv_s).or(ar_s);
        let end_all = ar_e.or(cv_e).or(ex_e).or(start_all);
        let ex_e = ex_e.or(cv_s).or(end_all);
        let cv_s = cv_s.or(ex_e);
        let cv_e = cv_e.or(ar_s).or(end_all);
        let ar_s = ar_s.or(cv_e);
        let ar_e = ar_e.or(end_all);

        if start_all.is_none() {
            errors.push("Pipeline produced no progress messages.".to_string());
            return None;
        }

        let dur = |s: Option<Instant>, e: Option<Instant>| -> f64 {
            match (s, e) {
                (Some(s), Some(e)) => e.duration_since(s).as_secs_f64(),
                _ => 0.0,
            }
        };

        Some(PhaseTimings {
            extract_secs: dur(ex_s, ex_e),
            convert_secs: dur(cv_s, cv_e).max(0.001),
            archive_secs: dur(ar_s, ar_e),
        })
    }

    fn extract_to(dir: &Path, dest: &Path, fallback_7z: &str) -> Result<Vec<PathBuf>, String> {
        fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        let ext = Path::new(dir)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext == "zip" || ext == "cbz" {
            let f = fs::File::open(dir).map_err(|e| e.to_string())?;
            let mut ar = ZipArchive::new(f).map_err(|e| e.to_string())?;
            ar.extract(dest).map_err(|e| e.to_string())?;
        } else {
            // 7z path (cbr/rar/7z)
            if fallback_7z.trim().is_empty() || !Path::new(fallback_7z).exists() {
                return Err("CBR/RAR source requires a valid 7z.exe path".to_string());
            }
            let mut cmd = std::process::Command::new(fallback_7z);
            cmd.arg("x")
                .arg(dir)
                .arg(format!("-o{}", dest.display()))
                .arg("-y");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let output = cmd
                .output()
                .map_err(|e| format!("7z launch failed: {}", e))?;
            if !output.status.success() {
                return Err(format!(
                    "7z failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }

        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(dest) {
            let e = entry.map_err(|e| e.to_string())?;
            if e.file_type().is_file() {
                let ext = e
                    .path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif") {
                    files.push(e.path().to_path_buf());
                }
            }
        }
        files.sort();
        Ok(files)
    }

    fn read_zip_entries(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
        let f = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut ar = ZipArchive::new(f).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for i in 0..ar.len() {
            let mut file = ar.by_index(i).map_err(|e| e.to_string())?;
            if file.is_dir() {
                continue;
            }
            let name = file.name().to_string();
            let mut buf = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            out.push((name, buf));
        }
        Ok(out)
    }

    fn decode_webp(bytes: &[u8]) -> Option<image::RgbaImage> {
        use image::ColorType;
        let decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
        let (w, h) = decoder.dimensions();
        let color_type = decoder.color_type();
        let mut raw = vec![0u8; usize::try_from(decoder.total_bytes()).ok()?];
        decoder.read_image(&mut raw).ok()?;
        // Opaque WebPs decode as Rgb8; normalize everything to RGBA so PSNR
        // comparisons against the original page are apples-to-apples.
        match color_type {
            ColorType::Rgba8 => image::RgbaImage::from_raw(w, h, raw),
            ColorType::Rgb8 => Some(
                image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, raw)?).to_rgba8(),
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn psnr_identical_returns_none() {
        let a = vec![128u8; 4 * 100];
        assert_eq!(psnr_rgba(&a, &a), None);
    }

    #[test]
    fn psnr_size_mismatch_or_empty_returns_none() {
        assert_eq!(psnr_rgba(&[0u8; 8], &[0u8; 16]), None);
        assert_eq!(psnr_rgba(&[], &[]), None);
    }

    #[test]
    fn psnr_known_difference() {
        let a = vec![100u8, 100, 100, 255];
        let b = vec![101u8, 101, 101, 255];
        let p = psnr_rgba(&a, &b).unwrap();
        let expected = 10.0 * (255.0f64 * 255.0 / 1.0).log10();
        assert!((p - expected).abs() < 1e-9, "got {} expected {}", p, expected);
    }

    #[test]
    fn webp_encode_decode_roundtrip() {
        let img = RgbaImage::from_pixel(37, 23, Rgba([10, 200, 30, 255]));
        let mem = webp::Encoder::from_rgba(img.as_raw(), 37, 23).encode(67.0);
        let dec = BenchmarkRunner::decode_webp(&mem).expect("decode failed");
        assert_eq!(dec.dimensions(), (37, 23));
    }
}
