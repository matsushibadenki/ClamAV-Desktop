use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: String,
    pub auto_update: bool,
    pub schedule_enabled: bool,
    pub schedule_hour: u32,
    pub max_file_mb: u32,
    #[serde(default = "default_retention")]
    pub download_retention_days: u32,
    #[serde(default = "default_cache_limit")]
    pub download_cache_mb: u32,
}
fn default_retention() -> u32 {
    30
}
fn default_cache_limit() -> u32 {
    1024
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "ja".into(),
            auto_update: false,
            schedule_enabled: false,
            schedule_hour: 12,
            max_file_mb: 100,
            download_retention_days: default_retention(),
            download_cache_mb: default_cache_limit(),
        }
    }
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Engine {
    pub path: Option<String>,
    pub version: Option<String>,
    pub definitions: bool,
    pub database_path: Option<String>,
    pub definitions_at: Option<String>,
    pub checked: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub path: String,
    pub signature: String,
    pub digest: Option<String>,
    pub quarantined: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub scanned: u64,
    pub errors: u64,
    pub current_file: String,
    pub findings: Vec<Finding>,
    pub message: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineItem {
    pub id: String,
    pub original_path: String,
    pub signature: String,
    pub digest: String,
    pub quarantined_at: String,
    pub mode: u32,
    pub status: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub at: String,
    pub code: String,
    pub detail: String,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub kind: String,
    pub target_id: Option<String>,
    pub stage: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub engine: Engine,
    pub scan: Option<Scan>,
    pub history: Vec<Scan>,
    pub quarantine: Vec<QuarantineItem>,
    pub settings: Settings,
    pub activity: Vec<Activity>,
    pub operation: Option<Operation>,
    pub activity_floating: bool,
    pub activity_dock: String,
    pub installer_ready: bool,
    pub data_dir: String,
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    #[test]
    fn previous_settings_keep_existing_choices_and_gain_cache_defaults() {
        let old = r#"{"language":"en","autoUpdate":true,"scheduleEnabled":false,"scheduleHour":8,"maxFileMb":200}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(settings.language, "en");
        assert_eq!(settings.max_file_mb, 200);
        assert_eq!(settings.download_retention_days, 30);
        assert_eq!(settings.download_cache_mb, 1024);
        let decoded: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(decoded.download_retention_days, 30);
    }
}
