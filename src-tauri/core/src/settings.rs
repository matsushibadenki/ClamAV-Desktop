//! One settings validation/persistence path for local GUI and worker clients.
use crate::{
    model::Settings,
    store::{err, Core, Result},
};

pub fn save(core: &Core, settings: Settings) -> Result<()> {
    if !["ja", "en", "zh-CN"].contains(&settings.language.as_str())
        || settings.schedule_hour > 23
        || !(1..=4096).contains(&settings.max_file_mb)
        || !(1..=365).contains(&settings.download_retention_days)
        || !(257..=16384).contains(&settings.download_cache_mb)
    {
        return Err("invalid_settings".into());
    }
    // Keep DB writes and document updates ordered even across concurrent GUI requests.
    let _gate = core.settings_gate.lock().unwrap();
    core.db
        .lock()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO settings(id,json) VALUES (1,?1)",
            [serde_json::to_string(&settings).map_err(err)?],
        )
        .map_err(err)?;
    core.document.lock().unwrap().settings = settings;
    drop(_gate);
    core.publish();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_values_and_failed_database_write_preserve_settings() {
        let root = std::path::PathBuf::from("/private/tmp")
            .join(format!("clamav-settings-{}", uuid::Uuid::new_v4()));
        let core = Core::new(root.clone()).unwrap();
        let original = serde_json::to_value(core.snapshot().settings).unwrap();
        let invalid = [
            Settings {
                language: "fr".into(),
                ..Settings::default()
            },
            Settings {
                schedule_hour: 24,
                ..Settings::default()
            },
            Settings {
                max_file_mb: 0,
                ..Settings::default()
            },
            Settings {
                max_file_mb: 4097,
                ..Settings::default()
            },
            Settings {
                download_retention_days: 0,
                ..Settings::default()
            },
            Settings {
                download_retention_days: 366,
                ..Settings::default()
            },
            Settings {
                download_cache_mb: 256,
                ..Settings::default()
            },
            Settings {
                download_cache_mb: 16385,
                ..Settings::default()
            },
        ];
        for settings in invalid {
            assert_eq!(save(&core, settings).unwrap_err(), "invalid_settings");
        }
        core.db
            .lock()
            .unwrap()
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        assert!(save(
            &core,
            Settings {
                language: "en".into(),
                ..Settings::default()
            }
        )
        .is_err());
        assert_eq!(
            serde_json::to_value(core.snapshot().settings).unwrap(),
            original
        );
        drop(core);
        let reopened = Core::new(root.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(reopened.snapshot().settings).unwrap(),
            original
        );
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
