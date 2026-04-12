#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 600.0])
            .with_min_inner_size([700.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Comic Converter Rust",
        options,
        Box::new(|_cc| Ok(Box::new(ComicConverterApp::new()))),
    )
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
    }

    fn start_processing(&mut self) {
        if self.settings.source_items.is_empty()
            || self.settings.temp_folder.trim().is_empty()
            || self.settings.final_folder.trim().is_empty()
        {
            self.log_msg("ERROR: Please add items to process and select Temp and Final folders.");
            return;
        }

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
            threads: s.threads.parse().unwrap_or_else(|_| num_cpus::get()),
            resize: s.resize,
            quality: s.quality.parse().unwrap_or(67.0),
            delete_source: s.delete_source,
            copy_final: s.copy_final,
            trim_pages: s.trim_pages,
            smart_trim_pages: s.smart_trim_pages,
            trim_min_size: s.trim_min_size.parse::<f64>().unwrap_or(75.0) / 100.0,
            smart_trim_threshold: s.smart_trim_threshold.parse::<f64>().unwrap_or(97.0) / 100.0,
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
}

impl eframe::App for ComicConverterApp {
    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {}

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut msgs = Vec::new();
        if let Some(rx) = &self.progress_rx {
            while let Ok(msg) = rx.try_recv() {
                msgs.push((msg.percentage, msg.message));
            }
            if !msgs.is_empty() {
                ctx.request_repaint();
            }
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

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.is_processing, egui::Button::new("Start Validation"))
                    .clicked()
                {
                    self.save_settings();
                    self.start_processing();
                }

                let pbar = egui::ProgressBar::new(self.progress as f32 / 100.0)
                    .text(format!("{}%", self.progress));
                ui.add(pbar);
            });
        });

        egui::SidePanel::left("left_panel").default_width(320.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Configuration");
                ui.separator();

                ui.checkbox(&mut self.settings.delete_source, "Delete Source");
                ui.checkbox(&mut self.settings.delete_temp, "Delete Temp");
                ui.checkbox(&mut self.settings.copy_final, "Copy Final");
                ui.checkbox(&mut self.settings.trim_pages, "Trim Pages");
                ui.checkbox(&mut self.settings.smart_trim_pages, "Smart Trim Pages");
                ui.checkbox(&mut self.settings.include_range_in_name, "Include Range In Name");

                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Threads:");
                    ui.text_edit_singleline(&mut self.settings.threads);
                });
                ui.horizontal(|ui| {
                    ui.label("Quality:");
                    ui.text_edit_singleline(&mut self.settings.quality);
                });
                ui.horizontal(|ui| {
                    ui.label("Resize (%):");
                    ui.text_edit_singleline(&mut self.settings.resize);
                });

                ui.separator();
                ui.label("Trim Settings:");
                ui.horizontal(|ui| {
                    ui.label("Min Size:");
                    ui.text_edit_singleline(&mut self.settings.trim_min_size);
                });
                ui.horizontal(|ui| {
                    ui.label("Threshold:");
                    ui.text_edit_singleline(&mut self.settings.smart_trim_threshold);
                });
                ui.horizontal(|ui| {
                    ui.label("Tolerance:");
                    ui.text_edit_singleline(&mut self.settings.smart_trim_tolerance);
                });

                ui.separator();
                ui.label("Zip Mode:");
                ui.radio_value(&mut self.settings.zip_mode, "single".to_string(), "Single");
                ui.radio_value(&mut self.settings.zip_mode, "individual".to_string(), "Individual");
                ui.radio_value(&mut self.settings.zip_mode, "none".to_string(), "None");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Source Items");
            ui.horizontal(|ui| {
                if ui.button("Add Folder").clicked() {
                    if let Some(folders) = FileDialog::new().pick_folders() {
                        for f in folders {
                            let path_str = f.to_string_lossy().to_string();
                            if !self.settings.source_items.contains(&path_str) {
                                self.settings.source_items.push(path_str);
                            }
                        }
                    }
                }
                if ui.button("Add Files").clicked() {
                    if let Some(files) = FileDialog::new().pick_files() {
                        for f in files {
                            let path_str = f.to_string_lossy().to_string();
                            if !self.settings.source_items.contains(&path_str) {
                                self.settings.source_items.push(path_str);
                            }
                        }
                    }
                }
                if ui.button("Clear").clicked() {
                    self.settings.source_items.clear();
                }
            });

            ui.group(|ui| {
                egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
                    let mut to_remove = None;
                    for (i, item) in self.settings.source_items.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.button("X").clicked() {
                                to_remove = Some(i);
                            }
                            ui.label(item);
                        });
                    }
                    if let Some(i) = to_remove {
                        self.settings.source_items.remove(i);
                    }
                });
            });

            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Temp Folder:");
                if ui.button("Browse").clicked() {
                    if let Some(folder) = FileDialog::new().pick_folder() {
                        self.settings.temp_folder = folder.to_string_lossy().to_string();
                    }
                }
            });
            ui.text_edit_singleline(&mut self.settings.temp_folder);

            ui.horizontal(|ui| {
                ui.label("Final Folder:");
                if ui.button("Browse").clicked() {
                    if let Some(folder) = FileDialog::new().pick_folder() {
                        self.settings.final_folder = folder.to_string_lossy().to_string();
                    }
                }
            });
            ui.text_edit_singleline(&mut self.settings.final_folder);

            ui.horizontal(|ui| {
                ui.label("7z.exe Path:");
                if ui.button("Browse").clicked() {
                    if let Some(file) = FileDialog::new().pick_file() {
                        self.settings.fallback_7z = file.to_string_lossy().to_string();
                    }
                }
            });
            ui.text_edit_singleline(&mut self.settings.fallback_7z);

            ui.separator();
            ui.heading("Log");

            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.log_text)
                            .desired_width(f32::INFINITY)
                            .desired_rows(10)
                            .interactive(false),
                    );
                });
        });
    }

    fn on_exit(&mut self) {
        self.save_settings();
    }
}
