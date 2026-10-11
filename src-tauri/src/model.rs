use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: String,
    pub auto_update: bool,
    pub schedule_enabled: bool,
    pub schedule_hour: u32,
    pub max_file_mb: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "ja".into(),
            auto_update: false,
            schedule_enabled: false,
            schedule_hour: 12,
            max_file_mb: 100,
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
