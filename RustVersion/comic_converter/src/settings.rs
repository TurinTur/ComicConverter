use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub source_items: Vec<String>,
    pub temp_folder: String,
    pub final_folder: String,
    pub fallback_7z: String,
    pub threads: String,
    pub resize: String,
    pub quality: String,
    pub delete_source: bool,
    pub delete_temp: bool,
    pub copy_final: bool,
    pub trim_pages: bool,
    pub smart_trim_pages: bool,
    pub trim_min_size: String,
    pub smart_trim_threshold: String,
    pub smart_trim_tolerance: String,
    pub zip_mode: String,
    pub include_range_in_name: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            source_items: Vec::new(),
            temp_folder: String::new(),
            final_folder: String::new(),
            fallback_7z: String::new(),
            threads: String::new(),
            resize: "100%".to_string(),
            quality: "67".to_string(),
            delete_source: false,
            delete_temp: true,
            copy_final: true,
            trim_pages: true,
            smart_trim_pages: false,
            trim_min_size: "75".to_string(),
            smart_trim_threshold: "97".to_string(),
            smart_trim_tolerance: "8".to_string(),
            zip_mode: "single".to_string(),
            include_range_in_name: true,
        }
    }
}

impl AppSettings {
    pub fn save(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let json = fs::read_to_string(path)?;
        let settings = serde_json::from_str(&json)?;
        Ok(settings)
    }
}
