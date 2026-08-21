#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod benchmark;
mod image_engine;
mod processor;
mod settings;

use eframe::egui;
use rfd::FileDialog;
use std::env;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use processor::{ComicProcessor, ProcessorContext, ProgressReport};
use settings::AppSettings;

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4C, 0x9E, 0xF8);
const ACCENT_DIM: egui::Color32 = egui::Color32::from_rgb(0x7F, 0xB8, 0xE8);
const BG_SIDE: egui::Color32 = egui::Color32::from_rgb(0x15, 0x17, 0x1C);
const BG_CENTRAL: egui::Color32 = egui::Color32::from_rgb(0x1A, 0x1D, 0x23);
const BG_BOTTOM: egui::Color32 = egui::Color32::from_rgb(0x10, 0x12, 0x16);
const CARD_FILL: egui::Color32 = egui::Color32::from_rgb(0x20, 0x24, 0x2B);
const CARD_STROKE: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x2F, 0x38);
const TEXT_MAIN: egui::Color32 = egui::Color32::from_rgb(0xD8, 0xDB, 0xDF);
const TEXT_WEAK: egui::Color32 = egui::Color32::from_rgb(0x8B, 0x91, 0x99);
const LOG_ERROR: egui::Color32 = egui::Color32::from_rgb(0xF2, 0x8B, 0x82);
const LOG_WARN: egui::Color32 = egui::Color32::from_rgb(0xF2, 0xC9, 0x4C);

fn main() -> Result<(), eframe::Error> {
    // Headless benchmark mode: `comic_converter.exe --benchmark <sources...> [options]`
    // Runs without the GUI so it can be scripted and timed cleanly.
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--benchmark") {
        benchmark::BenchmarkRunner::run_cli(&args);
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([860.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Comic Converter Rust",
        options,
        Box::new(|cc| {
            apply_style(&cc.egui_ctx);
            Ok(Box::new(ComicConverterApp::new()))
        }),
    )
}

fn apply_style(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();

    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(18.0),
    );
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(11.5));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));

    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 5.0);

    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG_SIDE;
    style.visuals.window_fill = BG_CENTRAL;
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(0x12, 0x14, 0x18);
    style.visuals.faint_bg_color = egui::Color32::from_rgb(0x1E, 0x22, 0x28);
    style.visuals.override_text_color = Some(TEXT_MAIN);
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    style.visuals.hyperlink_color = ACCENT;

    let radius = egui::CornerRadius::same(6);
    style.visuals.widgets.noninteractive.corner_radius = radius;
    style.visuals.widgets.inactive.corner_radius = radius;
    style.visuals.widgets.hovered.corner_radius = radius;
    style.visuals.widgets.active.corner_radius = radius;

    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(0x26, 0x2B, 0x33);
    style.visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(0x30, 0x37, 0x40);
    style.visuals.widgets.active.bg_fill = ACCENT;

    ctx.set_global_style(style);
}

struct ComicConverterApp {
    settings: AppSettings,
    settings_path: PathBuf,
    log_text: String,
    progress: usize,
    is_processing: bool,

    progress_rx: Option<Receiver<ProgressReport>>,
}

impl ComicConverterApp {
    fn new() -> Self {
        let mut base_dir = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        base_dir.pop();
        let settings_path = base_dir.join("settings.json");

        let mut settings = AppSettings::load(&settings_path).unwrap_or_default();

        if settings.temp_folder.is_empty() {
            settings.temp_folder = base_dir.join("temp").to_string_lossy().into_owned();
        }
        if settings.threads.is_empty() {
            settings.threads = num_cpus::get().max(1).to_string();
        }

        Self {
            settings,
            settings_path,
            log_text: String::new(),
            progress: 0,
            is_processing: false,
            progress_rx: None,
        }
    }

    fn save_settings(&self) {
        let _ = self.settings.save(&self.settings_path);
    }

    fn log_msg(&mut self, msg: &str) {
        let time = chrono::Local::now().format("%H:%M:%S");
        self.log_text.push_str(&format!("[{}] {}\n", time, msg));
        // Keep the log bounded so long runs don't grow memory indefinitely.
        if self.log_text.len() > 512 * 1024 {
            let cut = self.log_text.len() - 256 * 1024;
            let split = self.log_text[cut..].find('\n').map(|i| cut + i + 1).unwrap_or(cut);
            self.log_text.drain(..split);
        }
    }

    fn start_processing(&mut self) {
        if self.settings.source_items.is_empty()
            || self.settings.temp_folder.trim().is_empty()
            || self.settings.final_folder.trim().is_empty()
        {
            self.log_msg("ERROR: Please add items to process and select Temp and Final folders.");
            return;
        }

        let threads = match self.settings.threads.trim().parse::<usize>() {
            Ok(n) if n >= 1 => n,
            _ => {
                let n = num_cpus::get();
                self.log_msg(&format!("Invalid thread count '{}', using {}.", self.settings.threads, n));
                n
            }
        };
        let quality = match self.settings.quality.trim().parse::<f32>() {
            Ok(q) if (0.0..=100.0).contains(&q) => q,
            _ => {
                self.log_msg("Invalid quality value, using 67.");
                67.0
            }
        };

        self.is_processing = true;
        self.progress = 0;
        self.log_text.clear();
        self.log_msg("Process started...");

        let (tx, rx) = mpsc::channel();
        self.progress_rx = Some(rx);

        let s = self.settings.clone();

        let ctx = ProcessorContext {
            source_items: s.source_items,
            temp_folder: s.temp_folder,
            final_folder: s.final_folder,
            fallback_7z: s.fallback_7z,
            threads,
            resize: s.resize,
            quality,
            delete_source: s.delete_source,
            copy_final: s.copy_final,
            trim_pages: s.trim_pages,
            smart_trim_pages: s.smart_trim_pages,
            trim_min_size: s.trim_min_size.parse::<f64>().unwrap_or(75.0).clamp(0.0, 100.0) / 100.0,
            smart_trim_threshold: s.smart_trim_threshold.parse::<f64>().unwrap_or(97.0).clamp(0.0, 100.0) / 100.0,
            smart_trim_tolerance: s.smart_trim_tolerance.parse().unwrap_or(8.0),
            zip_mode: s.zip_mode,
            delete_temp: s.delete_temp,
            include_range_in_name: s.include_range_in_name,
        };

        thread::spawn(move || {
            let res = ComicProcessor::process(&ctx, tx.clone());
            if let Err(e) = res {
                let _ = tx.send(ProgressReport {
                    percentage: 100,
                    message: format!("FATAL ERROR: {}", e),
                });
            }
            let _ = tx.send(ProgressReport {
                percentage: 100,
                message: "DONE".to_string(),
            });
        });
    }

    fn poll_progress(&mut self, ctx: &egui::Context) {
        let mut msgs = Vec::new();
        if let Some(rx) = &self.progress_rx {
            while let Ok(msg) = rx.try_recv() {
                msgs.push((msg.percentage, msg.message));
            }
            if !msgs.is_empty() {
                ctx.request_repaint();
            }
        } else if self.is_processing {
            // Worker channel is gone but we never saw DONE (e.g. worker panicked);
            // recover the UI instead of staying locked forever.
            self.is_processing = false;
            self.log_msg("Processing stopped unexpectedly.");
            ctx.request_repaint();
        }

        let mut done = false;
        for (pct, msg) in msgs {
            if msg == "DONE" {
                done = true;
                continue;
            }
            self.progress = pct;
            self.log_msg(&msg);
        }

        if done {
            self.is_processing = false;
            self.progress_rx = None;
        }
    }

    fn action_bar(ui: &mut egui::Ui, app: &mut Self) {
        ui.horizontal(|ui| {
            let label = if app.is_processing { "Working..." } else { "Start" };
            let btn = egui::Button::new(egui::RichText::new(label).strong())
                .min_size(egui::vec2(130.0, 32.0))
                .fill(if app.is_processing {
                    egui::Color32::from_rgb(0x2A, 0x2F, 0x36)
                } else {
                    ACCENT
                });
            if ui.add_enabled(!app.is_processing, btn).clicked() {
                app.save_settings();
                app.start_processing();
            }
            if app.is_processing {
                ui.spinner();
            }

            let bar = egui::ProgressBar::new(app.progress as f32 / 100.0)
                .desired_height(20.0)
                .text(format!("{}%", app.progress));
            ui.add_sized(egui::vec2(ui.available_width(), 20.0), bar);
        });
    }

    fn options_panel(ui: &mut egui::Ui, app: &mut Self) {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                section_header(ui, "Options");
                checkbox_row(
                    ui,
                    &mut app.settings.delete_source,
                    "Delete Source if final size is smaller",
                    "Deletes the original archive/folder after conversion,\nbut only if the converted result is smaller than the source.",
                );
                checkbox_row(
                    ui,
                    &mut app.settings.delete_temp,
                    "Delete Temp Folder at end",
                    "Removes the whole temp working folder when processing finishes.",
                );
                checkbox_row(
                    ui,
                    &mut app.settings.copy_final,
                    "Copy final zip to Final Output Folder",
                    "Copies the resulting CBZ/zip files to the final output folder.",
                );
                checkbox_row(
                    ui,
                    &mut app.settings.trim_pages,
                    "Trim pages (remove borders)",
                    "Automatically crops white/black borders around each page.",
                );
                checkbox_row(
                    ui,
                    &mut app.settings.smart_trim_pages,
                    "Smart Trim (ignore small elements like page numbers)",
                    "Trims borders where \u{2265}97% of pixels match the background,\neven if a page number interrupts the edge.",
                );
                checkbox_row(
                    ui,
                    &mut app.settings.include_range_in_name,
                    "Include volume/chapter range in name",
                    "Adds the detected volume/chapter range to the output file name.",
                );

                egui::CollapsingHeader::new("Trim Settings")
                    .default_open(false)
                    .show(ui, |ui| {
                        field_row_tooltip(ui, "Min Size (%)", &mut app.settings.trim_min_size, "Minimum size after trim (default 75)");
                        field_row_tooltip(ui, "Smart Threshold (%)", &mut app.settings.smart_trim_threshold, "Percentage of background pixels (default 97)");
                        field_row_tooltip(ui, "Smart Tolerance (%)", &mut app.settings.smart_trim_tolerance, "Color distance tolerance (default 8)");
                    });

                section_header(ui, "Conversion Settings");
                field_row_tooltip(ui, "Threads", &mut app.settings.threads, "Number of parallel conversions");
                field_row_tooltip(ui, "Quality (1-100)", &mut app.settings.quality, "WebP Quality level");
                field_row_tooltip(ui, "Resize (%)", &mut app.settings.resize, "Image resize percentage (e.g., 100 keeps original size)");

                section_header(ui, "Zip Mode");
                radio_row(ui, &mut app.settings.zip_mode, "single", "Single (One CBZ for all)", "All processed items are packed into one single CBZ file.");
                radio_row(ui, &mut app.settings.zip_mode, "individual", "Individual (One CBZ per folder/file)", "Each folder/file becomes its own CBZ file.");
                radio_row(ui, &mut app.settings.zip_mode, "none", "None (Keep folder structure)", "No zipping; the converted folder structure is kept as-is.");
                ui.add_space(8.0);
            });
    }

    fn sources_section(ui: &mut egui::Ui, app: &mut Self) {
        ui.horizontal(|ui| {
            ui.heading("Sources");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Clear").clicked() {
                    app.settings.source_items.clear();
                }
                if ui.button("Add Files").clicked() {
                    if let Some(files) = FileDialog::new().pick_files() {
                        for f in files {
                            let path_str = f.to_string_lossy().to_string();
                            if !app.settings.source_items.contains(&path_str) {
                                app.settings.source_items.push(path_str);
                            }
                        }
                    }
                }
                if ui.button("Add Folder").clicked() {
                    if let Some(folders) = FileDialog::new().pick_folders() {
                        for f in folders {
                            let path_str = f.to_string_lossy().to_string();
                            if !app.settings.source_items.contains(&path_str) {
                                app.settings.source_items.push(path_str);
                            }
                        }
                    }
                }
            });
        });
        ui.add_space(2.0);

        egui::Frame::new()
            .fill(CARD_FILL)
            .stroke(egui::Stroke::new(1.0, CARD_STROKE))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(130.0)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if app.settings.source_items.is_empty() {
                            ui.label(
                                egui::RichText::new("No sources added yet.")
                                    .color(TEXT_WEAK)
                                    .size(13.0),
                            );
                        }
                        let mut remove: Option<usize> = None;
                        for (i, item) in app.settings.source_items.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("X").clicked() {
                                    remove = Some(i);
                                }
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(item).size(13.0),
                                    )
                                    .truncate(),
                                );
                            });
                        }
                        if let Some(i) = remove {
                            app.settings.source_items.remove(i);
                        }
                    });
            });
    }

    fn folders_section(ui: &mut egui::Ui, app: &mut Self) {
        section_header(ui, "Folders");

        let browse_size = egui::vec2(92.0, 22.0);
        let gap = ui.style().spacing.item_spacing.x;

        let mut browse_temp = false;
        let mut browse_final = false;
        let mut browse_7z = false;

        // Field width is derived from the space left after the label, minus the
        // fixed-size Browse button, so the button always ends flush with the
        // panel's right edge regardless of font metrics or window size.
        let path_row = |ui: &mut egui::Ui,
                            label: &str,
                            value: &mut String,
                            tooltip: &str,
                            browse: &mut bool| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    egui::vec2(90.0, 20.0),
                    egui::Label::new(egui::RichText::new(label).color(TEXT_WEAK)),
                );
                let field_w =
                    ui.available_width() - browse_size.x - gap;
                ui.add(
                    egui::TextEdit::singleline(value).desired_width(field_w.max(120.0)),
                )
                .on_hover_text(tooltip);
                if ui
                    .add_sized(browse_size, egui::Button::new("Browse..."))
                    .clicked()
                {
                    *browse = true;
                }
            });
        };

        path_row(ui, "Temp Folder", &mut app.settings.temp_folder, "Folder where extraction and conversion happens", &mut browse_temp);
        path_row(ui, "Final Folder", &mut app.settings.final_folder, "Where the final CBZ files will be copied", &mut browse_final);
        path_row(ui, "7z.exe Path (Opt)", &mut app.settings.fallback_7z, "Optional path to 7z.exe; needed for CBR/RAR/7z archives.\nLeave empty to use the built-in zip handling.", &mut browse_7z);

        if browse_temp {
            if let Some(folder) = FileDialog::new().pick_folder() {
                app.settings.temp_folder = folder.to_string_lossy().to_string();
            }
        }
        if browse_final {
            if let Some(folder) = FileDialog::new().pick_folder() {
                app.settings.final_folder = folder.to_string_lossy().to_string();
            }
        }
        if browse_7z {
            if let Some(file) = FileDialog::new().pick_file() {
                app.settings.fallback_7z = file.to_string_lossy().to_string();
            }
        }
    }

    fn log_section(ui: &mut egui::Ui, app: &mut Self) {
        section_header(ui, "Log");
        let line_height = 17.0;
        let total = app.log_text.lines().count();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, line_height, total, |ui, range| {
                for line in app
                    .log_text
                    .lines()
                    .skip(range.start)
                    .take(range.end - range.start)
                {
                    let color = if line.contains("ERROR") || line.contains("FATAL") {
                        LOG_ERROR
                    } else if line.contains("WARNING") || line.contains("WARN") {
                        LOG_WARN
                    } else {
                        TEXT_MAIN
                    };
                    ui.label(egui::RichText::new(line).monospace().color(color));
                }
            });
    }
}

impl eframe::App for ComicConverterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        egui::Panel::bottom("action_bar")
            .frame(
                egui::Frame::new()
                    .fill(BG_BOTTOM)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show_inside(ui, |ui| {
                Self::action_bar(ui, self);
            });

        egui::Panel::left("options_panel")
            .default_size(300.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_SIDE)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show_inside(ui, |ui| {
                Self::options_panel(ui, self);
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG_CENTRAL)
                    .inner_margin(egui::Margin {
                        left: 16,
                        right: 16,
                        top: 12,
                        bottom: 12,
                    }),
            )
            .show_inside(ui, |ui| {
                self.poll_progress(&ctx);
                Self::sources_section(ui, self);
                ui.add_space(6.0);
                Self::folders_section(ui, self);
                ui.add_space(6.0);
                Self::log_section(ui, self);
            });
    }

    fn on_exit(&mut self) {
        self.save_settings();
    }
}

fn section_header(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .size(11.0)
            .strong()
            .extra_letter_spacing(1.2)
            .color(ACCENT_DIM),
    );
    ui.separator();
}

fn checkbox_row(ui: &mut egui::Ui, checked: &mut bool, text: &str, tooltip: &str) {
    ui.checkbox(checked, text).on_hover_text(tooltip);
}

fn radio_row(ui: &mut egui::Ui, value: &mut String, option: &str, text: &str, tooltip: &str) {
    ui.radio_value(value, option.to_string(), text)
        .on_hover_text(tooltip);
}

fn field_row_tooltip(ui: &mut egui::Ui, label: &str, value: &mut String, tooltip: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(120.0, 20.0),
            egui::Label::new(egui::RichText::new(label).color(TEXT_WEAK)),
        )
        .on_hover_text(tooltip);
        ui.add_sized(
            egui::vec2(ui.available_width(), 20.0),
            egui::TextEdit::singleline(value),
        )
        .on_hover_text(tooltip);
    });
}
