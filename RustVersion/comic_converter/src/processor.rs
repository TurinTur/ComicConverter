use crate::image_engine::ImageEngine;
use rayon::prelude::*;
use regex::Regex;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use zip::{ZipArchive, ZipWriter};
use zip::write::SimpleFileOptions;

pub struct ProcessorContext {
    pub source_items: Vec<String>,
    pub temp_folder: String,
    pub final_folder: String,
    pub fallback_7z: String,
    pub threads: usize,
    pub resize: String,
    pub quality: f32,
    pub delete_source: bool,
    pub copy_final: bool,
    pub trim_pages: bool,
    pub smart_trim_pages: bool,
    pub trim_min_size: f64,
    pub smart_trim_threshold: f64,
    pub smart_trim_tolerance: f64,
    pub zip_mode: String,
    pub delete_temp: bool,
    pub include_range_in_name: bool,
}

pub struct ProgressReport {
    pub percentage: usize,
    pub message: String,
}

pub struct ComicProcessor;

impl ComicProcessor {
    pub fn process(ctx: &ProcessorContext, progress_tx: std::sync::mpsc::Sender<ProgressReport>) -> Result<(), String> {
        let report = |pct: usize, msg: String| {
            let _ = progress_tx.send(ProgressReport { percentage: pct, message: msg });
        };

        report(0, "Process started...".to_string());
        
        // Calculate initial size
        let mut initial_size: u64 = 0;
        for item in &ctx.source_items {
            let p = Path::new(item);
            if p.is_dir() {
                initial_size += Self::dir_size(p);
            } else if p.is_file() {
                if let Ok(m) = fs::metadata(p) {
                    initial_size += m.len();
                }
            }
        }
        report(0, format!("Initial size: {} MB", initial_size / (1024 * 1024)));

        // Step 1: Extract archives
        report(5, "Step 1 & 2: Extracting archives...".to_string());
        let work_items = Self::extract_archives_to_temp(&ctx, &report)?;

        // Step 3: Replicate structure
        report(20, "Step 3: Creating target folder structure...".to_string());
        Self::replicate_folder_structure(&work_items, &ctx.temp_folder)?;

        // Step 4: Convert
        report(25, "Step 4: Converting images to WebP...".to_string());
        Self::convert_images(&ctx, &work_items, &report)?;

        let final_temp_size = Self::dir_size(Path::new(&ctx.temp_folder));
        report(80, format!("Conversion complete. Target temp size: {} MB", final_temp_size / (1024 * 1024)));

        // Step 5: Archive
        report(85, "Step 5: Archiving results to CBZ...".to_string());
        let dest_dir = if ctx.copy_final { &ctx.final_folder } else { &ctx.temp_folder };
        fs::create_dir_all(dest_dir).map_err(|e| format!("Failed to create final dir: {}", e))?;
        
        let final_archive_size = Self::archive_results(&ctx, dest_dir)?;

        let mut size_report = format!("Final converted size: {} MB", final_archive_size / (1024 * 1024));
        if initial_size > 0 && final_archive_size < initial_size {
            let reduction = ((initial_size as f64 - final_archive_size as f64) / initial_size as f64) * 100.0;
            size_report = format!("{}, {}% size reduction improvement", size_report, reduction.round());
        } else if initial_size > 0 && final_archive_size > initial_size {
            let increase = ((final_archive_size as f64 - initial_size as f64) / initial_size as f64) * 100.0;
            size_report = format!("{}, {}% size increase", size_report, increase.round());
        }
        report(95, size_report);

        if ctx.delete_source {
            if final_archive_size <= initial_size && final_archive_size > 0 {
                report(98, "Step 6: Final size is smaller. Deleting source files...".to_string());
                for item in &ctx.source_items {
                    let p = Path::new(item);
                    if p.is_dir() {
                        let _ = fs::remove_dir_all(p);
                    } else if p.is_file() {
                        let _ = fs::remove_file(p);
                    }
                }
            } else {
                report(98, "Step 6: Final size is NOT smaller. Keeping source files.".to_string());
            }
        }

        if ctx.delete_temp {
            report(99, "Cleaning up temp folder...".to_string());
            let _ = Self::delete_directory_contents(Path::new(&ctx.temp_folder));
        }

        report(100, "Process completed!".to_string());
        Ok(())
    }

    fn extract_archives_to_temp<F>(ctx: &ProcessorContext, report: &F) -> Result<Vec<String>, String>
    where F: Fn(usize, String) {
        let exts = vec!["zip", "7z", "cbz", "cbr", "rar"];
        let mut results = Vec::new();
        let extraction_root = Path::new(&ctx.temp_folder).join("_extracted");
        let use_7z = !ctx.fallback_7z.trim().is_empty() && Path::new(&ctx.fallback_7z).exists();

        for item in &ctx.source_items {
            let p = Path::new(item);
            if p.is_dir() {
                if let Ok(entries) = walkdir::WalkDir::new(p).into_iter().collect::<Result<Vec<_>, _>>() {
                    for entry in entries {
                        if entry.file_type().is_file() {
                            if let Some(ext) = entry.path().extension().and_then(|s| s.to_str()) {
                                if exts.contains(&ext.to_lowercase().as_str()) {
                                    let sub_dest = extraction_root.join(entry.path().file_stem().unwrap());
                                    Self::extract_archive(entry.path(), &sub_dest, use_7z, &ctx.fallback_7z, report)?;
                                    if ctx.delete_source { let _ = fs::remove_file(entry.path()); }
                                }
                            }
                        }
                    }
                }
                results.push(item.clone());
            } else if p.is_file() {
                if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                    if exts.contains(&ext.to_lowercase().as_str()) {
                        let sub_dest = extraction_root.join(p.file_stem().unwrap());
                        Self::extract_archive(p, &sub_dest, use_7z, &ctx.fallback_7z, report)?;
                        results.push(sub_dest.to_string_lossy().to_string());
                        if ctx.delete_source { let _ = fs::remove_file(p); }
                    } else {
                        results.push(item.clone());
                    }
                } else {
                    results.push(item.clone());
                }
            }
        }
        Ok(results)
    }

    fn extract_archive<F>(file: &Path, dest_folder: &Path, use_7z: bool, fallback_7z: &str, report: &F) -> Result<(), String> 
    where F: Fn(usize, String) {
        fs::create_dir_all(dest_folder).map_err(|e| format!("Failed to create {}: {}", dest_folder.display(), e))?;
        report(5, format!("Extracting: {}", file.file_name().unwrap_or_default().to_string_lossy()));

        if use_7z {
            let mut cmd = Command::new(fallback_7z);
            cmd.arg("x").arg(file).arg(&format!("-o{}", dest_folder.display())).arg("-y");
            let _ = cmd.output().map_err(|e| format!("7z extraction failed: {}", e))?;
        } else {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if ext == "zip" || ext == "cbz" {
                let f = fs::File::open(file).map_err(|e| e.to_string())?;
                let mut archive = ZipArchive::new(f).map_err(|e| e.to_string())?;
                archive.extract(dest_folder).map_err(|e| e.to_string())?;
            } else {
                report(5, format!("Cannot extract {} without 7z.exe configured.", file.display()));
            }
        }
        Ok(())
    }

    fn replicate_folder_structure(items: &[String], target_base: &str) -> Result<(), String> {
        let base = Path::new(target_base);
        fs::create_dir_all(base).map_err(|e| e.to_string())?;

        for item in items {
            let p = Path::new(item);
            if p.is_dir() {
                let root_name = p.file_name().unwrap();
                let target_root = base.join(root_name);
                fs::create_dir_all(&target_root).map_err(|e| e.to_string())?;

                if let Ok(entries) = walkdir::WalkDir::new(p).into_iter().collect::<Result<Vec<_>, _>>() {
                    for entry in entries {
                        if entry.file_type().is_dir() {
                            if let Ok(relative) = entry.path().strip_prefix(p) {
                                let _ = fs::create_dir_all(target_root.join(relative));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn convert_images<F>(ctx: &ProcessorContext, items: &[String], report: &F) -> Result<(), String>
    where F: Fn(usize, String) + Sync + Send {
        let image_exts = vec!["jpg", "jpeg", "png", "bmp", "webp", "gif"];
        let mut tasks: Vec<(PathBuf, PathBuf)> = Vec::new();

        for item in items {
            let p = Path::new(item);
            if p.is_dir() {
                let root_name = p.file_name().unwrap();
                if let Ok(entries) = walkdir::WalkDir::new(p).into_iter().collect::<Result<Vec<_>, _>>() {
                    for entry in entries {
                        if entry.file_type().is_file() {
                            if let Some(ext) = entry.path().extension().and_then(|s| s.to_str()) {
                                if image_exts.contains(&ext.to_lowercase().as_str()) {
                                    if let Ok(relative) = entry.path().strip_prefix(p) {
                                        let dest = Path::new(&ctx.temp_folder).join(root_name).join(relative).with_extension("webp");
                                        tasks.push((entry.path().to_path_buf(), dest));
                                    }
                                }
                            }
                        }
                    }
                }
            } else if p.is_file() {
                if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                    if image_exts.contains(&ext.to_lowercase().as_str()) {
                        let dest = Path::new(&ctx.temp_folder).join(p.file_name().unwrap()).with_extension("webp");
                        tasks.push((p.to_path_buf(), dest));
                    }
                }
            }
        }

        let total = tasks.len();
        if total == 0 { return Ok(()); }

        let completed = AtomicUsize::new(0);
        let resize_percentage = if ctx.resize.trim() != "100%" && ctx.resize.ends_with('%') {
            ctx.resize.trim_end_matches('%').parse::<f32>().ok()
        } else {
            None
        };

        // Create rayon pool
        let pool = rayon::ThreadPoolBuilder::new().num_threads(ctx.threads).build().map_err(|e| e.to_string())?;

        pool.install(|| {
            tasks.par_iter().for_each(|(source, dest)| {
                if let Some(parent) = dest.parent() {
                    let _ = fs::create_dir_all(parent);
                }

                let res = ImageEngine::process_image(
                    source,
                    dest,
                    ctx.trim_pages,
                    ctx.smart_trim_pages,
                    ctx.trim_min_size,
                    ctx.smart_trim_threshold,
                    ctx.smart_trim_tolerance,
                    resize_percentage,
                    ctx.quality,
                );

                if res.is_err() {
                    let _ = fs::copy(source, dest.with_extension(source.extension().unwrap_or_default()));
                }

                let c = completed.fetch_add(1, Ordering::SeqCst) + 1;
                if c % 5 == 0 || c == total {
                    let pct = 25 + ((c as f64 / total as f64) * 55.0) as usize;
                    report(pct, format!("Converted {} of {} files", c, total));
                }
            });
        });

        Ok(())
    }

    fn archive_results(ctx: &ProcessorContext, dest_dir: &str) -> Result<u64, String> {
        let mut total_size = 0;
        let temp_path = Path::new(&ctx.temp_folder);

        let mut items = Vec::new();
        if let Ok(entries) = fs::read_dir(temp_path) {
            for entry in entries.flatten() {
                if entry.file_name() != "_extracted" {
                    items.push(entry.path());
                }
            }
        }

        if ctx.zip_mode == "single" {
            let names: Vec<String> = items.iter().filter_map(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).collect();
            let base_name = Self::get_smart_base_name(&names, ctx.include_range_in_name);
            let zip_path = Self::get_unique_file_path(dest_dir, &format!("{}.cbz", base_name));
            Self::zip_dir(&ctx.temp_folder, &zip_path, Some("_extracted"))?;
            total_size += fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
        } else if ctx.zip_mode == "individual" {
            for item in items {
                if item.is_dir() {
                    let zip_path = Self::get_unique_file_path(dest_dir, &format!("{}.cbz", item.file_name().unwrap().to_string_lossy()));
                    Self::zip_dir(&item.to_string_lossy(), &zip_path, None)?;
                    total_size += fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
                } else if item.is_file() {
                    let root_name = item.file_stem().unwrap().to_string_lossy();
                    let zip_path = Self::get_unique_file_path(dest_dir, &format!("{}.cbz", root_name));
                    Self::zip_single_file(&item, &zip_path)?;
                    total_size += fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
                }
            }
        } else {
            // mode = none
            let names: Vec<String> = items.iter().filter_map(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).collect();
            let base_name = Self::get_smart_base_name(&names, ctx.include_range_in_name);
            let target_path = Self::get_unique_folder_path(dest_dir, &base_name);
            Self::copy_dir(temp_path, Path::new(&target_path), Some("_extracted"))?;
            total_size += Self::dir_size(Path::new(&target_path));
        }

        Ok(total_size)
    }

    fn zip_dir(src: &str, dest: &str, exclude_root: Option<&str>) -> Result<(), String> {
        let file = fs::File::create(dest).map_err(|e| format!("Zip Error: {}", e))?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        
        let path = Path::new(src);
        if let Ok(entries) = walkdir::WalkDir::new(path).into_iter().collect::<Result<Vec<_>, _>>() {
            for entry in entries {
                let p = entry.path();
                let name = p.strip_prefix(path).unwrap().to_str().unwrap().replace("\\", "/");
                if name.is_empty() { continue; }
                
                if let Some(ex) = exclude_root {
                    if name.starts_with(ex) { continue; }
                }

                if p.is_file() {
                    zip.start_file(name, options).map_err(|e| e.to_string())?;
                    let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
                    std::io::copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
                } else if !name.is_empty() {
                    zip.add_directory(name, options).map_err(|e| e.to_string())?;
                }
            }
        }
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn zip_single_file(src: &Path, dest: &str) -> Result<(), String> {
        let file = fs::File::create(dest).map_err(|e| e.to_string())?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        let mut f = fs::File::open(src).map_err(|e| e.to_string())?;
        io::copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn copy_dir(src: &Path, dest: &Path, exclude_root: Option<&str>) -> Result<(), String> {
        fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        if let Ok(entries) = walkdir::WalkDir::new(src).into_iter().collect::<Result<Vec<_>, _>>() {
            for entry in entries {
                let p = entry.path();
                if p.is_file() {
                    let rel = p.strip_prefix(src).unwrap().to_string_lossy().to_string();
                    if let Some(ex) = exclude_root {
                        if rel.starts_with(ex) { continue; }
                    }
                    let tgt = dest.join(rel);
                    fs::create_dir_all(tgt.parent().unwrap()).unwrap();
                    fs::copy(p, tgt).unwrap();
                }
            }
        }
        Ok(())
    }

    fn dir_size(dir: &Path) -> u64 {
        walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .sum()
    }

    fn delete_directory_contents(dir: &Path) -> io::Result<()> {
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let p = entry?.path();
                if p.is_dir() {
                    fs::remove_dir_all(p)?;
                } else {
                    fs::remove_file(p)?;
                }
            }
        }
        Ok(())
    }

    fn get_unique_folder_path(dest_dir: &str, folder_name: &str) -> String {
        let mut full = Path::new(dest_dir).join(folder_name);
        let mut i = 1;
        while full.exists() {
            full = Path::new(dest_dir).join(format!("{}_{:03}", folder_name, i));
            i += 1;
        }
        full.to_string_lossy().to_string()
    }

    fn get_unique_file_path(dest_dir: &str, file_name: &str) -> String {
        let mut full = Path::new(dest_dir).join(file_name);
        let ext = full.extension().unwrap_or_default().to_string_lossy().into_owned();
        let name = full.file_stem().unwrap().to_string_lossy().into_owned();
        let ext_str = if ext.is_empty() { String::new() } else { format!(".{}", ext) };
        let mut i = 1;
        while full.exists() {
            full = Path::new(dest_dir).join(format!("{}_{:03}{}", name, i, ext_str));
            i += 1;
        }
        full.to_string_lossy().to_string()
    }

    fn get_smart_base_name(items: &[String], include_range: bool) -> String {
        let mut list = items.to_vec();
        list.sort();
        if list.is_empty() { return "archive".to_string() }

        let pattern = Regex::new(r"(?i)^(.*?)(?:[\s\-_.\(\[\]]*(?:v(?:ol)?\.?\s*\d+|ch(?:ap)?\.?\s*\d+|\(?\d{4}\)?))").unwrap();
        
        if list.len() == 1 {
            if let Some(caps) = pattern.captures(&list[0]) {
                let matched = caps.get(1).unwrap().as_str().trim_matches(&[' ', '-', '_', '.', '('] as &[_]);
                if !matched.is_empty() { return matched.to_string(); }
            }
            return Path::new(&list[0]).file_stem().unwrap().to_string_lossy().to_string();
        }

        let first = &list[0];
        let last = &list[list.len()-1];

        if let (Some(cap_f), Some(cap_l)) = (pattern.captures(first), pattern.captures(last)) {
            let pref_f = cap_f.get(1).unwrap().as_str().trim_matches(&[' ', '-', '_', '.', '('] as &[_]);
            let pref_l = cap_l.get(1).unwrap().as_str().trim_matches(&[' ', '-', '_', '.', '('] as &[_]);
            
            if !pref_f.is_empty() && pref_f.eq_ignore_ascii_case(pref_l) {
                if !include_range { return pref_f.to_string() }
                
                // Get ranges (quick extract volume/number part)
                let marker_f = first[pref_f.len()..].trim_matches(&[' ', '-', '_', '.', '(', ')'] as &[_]);
                let marker_l = last[pref_l.len()..].trim_matches(&[' ', '-', '_', '.', '(', ')'] as &[_]);
                
                if !marker_f.is_empty() && !marker_l.is_empty() && marker_f != marker_l {
                    return format!("{} {}-{}", pref_f, marker_f, marker_l);
                }
                return pref_f.to_string();
            }
        }

        if let Some(caps) = pattern.captures(first) {
            let pref = caps.get(1).unwrap().as_str().trim_matches(&[' ', '-', '_', '.', '('] as &[_]);
            if !pref.is_empty() { return pref.to_string(); }
        }
        Path::new(first).file_stem().unwrap().to_string_lossy().to_string()
    }
}
