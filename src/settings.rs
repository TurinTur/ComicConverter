use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
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
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let json = fs::read_to_string(path)?;
        let settings = serde_json::from_str(&json)?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_preserves_fields() {
        let mut s = AppSettings::default();
        s.source_items = vec!["C:\\comics".to_string()];
        s.quality = "80".to_string();
        s.trim_pages = false;

        let json = serde_json::to_string(&s).unwrap();
        let back: AppSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(back.source_items, s.source_items);
        assert_eq!(back.quality, "80");
        assert!(!back.trim_pages);
        assert_eq!(back.zip_mode, "single");
        assert!(back.delete_temp);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let partial = r#"{"source_items": ["a.cbz"]}"#;
        let loaded: AppSettings = serde_json::from_str(partial).unwrap();

        assert_eq!(loaded.source_items, vec!["a.cbz".to_string()]);
        assert_eq!(loaded.resize, "100%");
        assert_eq!(loaded.quality, "67");
        assert_eq!(loaded.zip_mode, "single");
        assert!(loaded.delete_temp);
        assert!(loaded.copy_final);
        assert!(loaded.trim_pages);
        assert!(!loaded.smart_trim_pages);
        assert_eq!(loaded.trim_min_size, "75");
        assert_eq!(loaded.smart_trim_threshold, "97");
        assert_eq!(loaded.smart_trim_tolerance, "8");
        assert!(loaded.include_range_in_name);
    }

    #[test]
    fn empty_object_loads_full_defaults() {
        let loaded: AppSettings = serde_json::from_str("{}").unwrap();
        let expected = AppSettings::default();
        assert_eq!(loaded.resize, expected.resize);
        assert_eq!(loaded.zip_mode, expected.zip_mode);
        assert_eq!(loaded.threads, expected.threads);
    }
}
