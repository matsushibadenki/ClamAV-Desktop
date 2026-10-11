mod details;
mod downloads;
mod engine;
mod file_transfer;
mod installer;
mod model;
mod process;
mod quarantine;
mod renderer;
mod scanner;
mod store;
use model::*;
use std::sync::atomic::Ordering;
use store::*;
use tauri::{AppHandle, Manager, WebviewWindow};
fn main_window(window: &WebviewWindow) -> Result<()> {
    if window.label() != "main" {
        return Err("main_window_required".into());
    }
    Ok(())
}
#[tauri::command]
fn get_snapshot(app: AppHandle) -> Snapshot {
    core(&app).snapshot()
}
#[tauri::command]
async fn refresh_engine(app: AppHandle) -> Result<Engine> {
    engine::inspect(&app).await
}
#[tauri::command]
async fn prepare_installer(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    installer::prepare(app).await
}
#[tauri::command]
async fn open_installer(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    installer::open(&app).await
}
#[tauri::command]
async fn update_definitions(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    engine::update(app).await
}
#[tauri::command]
async fn start_scan(
    app: AppHandle,
    window: WebviewWindow,
    kind: String,
    target: Option<String>,
) -> Result<String> {
    main_window(&window)?;
    scanner::start(app, kind, target).await
}
#[tauri::command]
fn stop_scan(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    core(&app).cancel.store(true, Ordering::Relaxed);
    Ok(())
}
#[tauri::command]
async fn choose_folder(window: WebviewWindow) -> Result<Option<String>> {
    main_window(&window)?;
    Ok(rfd::AsyncFileDialog::new()
        .pick_folder()
        .await
        .map(|p| p.path().to_string_lossy().into()))
}
#[tauri::command]
async fn quarantine_file(app: AppHandle, window: WebviewWindow, id: String) -> Result<()> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || quarantine::quarantine(&app, &id))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn restore_file(app: AppHandle, window: WebviewWindow, id: String) -> Result<()> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || quarantine::restore(&app, &id))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn recover_file_transfers(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let c = core(&app);
        let _gate = c.file_gate.lock().unwrap();
        quarantine::refresh_items(&app)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn get_download_cache(app: AppHandle) -> Result<downloads::CacheReport> {
    tauri::async_runtime::spawn_blocking(move || downloads::report(&core(&app)))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn cleanup_download_cache(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<downloads::CacheReport> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || downloads::cleanup(&app))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn get_quarantine_detail(app: AppHandle, id: String) -> Result<details::Detail> {
    tauri::async_runtime::spawn_blocking(move || details::get(&core(&app), &id))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn verify_quarantine_integrity(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<details::Detail> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        details::verify(&app, &id)?;
        details::get(&core(&app), &id)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
fn stop_integrity_check(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    let c = core(&app);
    {
        let mut d = c.document.lock().unwrap();
        if let Some(op) = d
            .operation
            .as_mut()
            .filter(|op| op.kind == "integrity" && op.stage != "finishingIntegrity")
        {
            c.integrity_cancel.store(true, Ordering::Relaxed);
            op.stage = "stoppingIntegrity".into();
        }
    }
    c.publish(&app);
    Ok(())
}
#[tauri::command]
async fn export_quarantine(app: AppHandle, window: WebviewWindow) -> Result<Option<String>> {
    main_window(&window)?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_file_name("clamav-quarantine.json")
        .add_filter("JSON", &["json"])
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let destination = file.path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let c = core(&app);
        let bytes = serde_json::to_vec_pretty(&details::export(&c)?).map_err(err)?;
        details::write_export(&c, &destination, &bytes)?;
        Ok(Some(destination.to_string_lossy().into()))
    })
    .await
    .map_err(err)?
}
#[tauri::command]
fn save_settings(app: AppHandle, window: WebviewWindow, settings: Settings) -> Result<()> {
    main_window(&window)?;
    if !["ja", "en", "zh-CN"].contains(&settings.language.as_str())
        || settings.schedule_hour > 23
        || !(1..=4096).contains(&settings.max_file_mb)
        || !(1..=365).contains(&settings.download_retention_days)
        || !(257..=16384).contains(&settings.download_cache_mb)
    {
        return Err("invalid_settings".into());
    }
    let c = core(&app);
    c.db.lock()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO settings(id,json) VALUES (1,?1)",
            [serde_json::to_string(&settings).map_err(err)?],
        )
        .map_err(err)?;
    c.document.lock().unwrap().settings = settings;
    c.publish(&app);
    Ok(())
}
#[tauri::command]
fn set_activity_panel(app: AppHandle, floating: bool, side: Option<String>) -> Result<()> {
    renderer::Renderer::activity_panel(&app, floating, side)
}
#[tauri::command]
async fn export_history(app: AppHandle, window: WebviewWindow) -> Result<Option<String>> {
    main_window(&window)?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_file_name("clamav-history.json")
        .add_filter("JSON", &["json"])
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec_pretty(&core(&app).snapshot().history).map_err(err)?;
    tokio::fs::write(file.path(), bytes).await.map_err(err)?;
    Ok(Some(file.path().to_string_lossy().into()))
}
#[tauri::command]
fn open_privacy_settings(window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .spawn()
        .map_err(err)?;
    Ok(())
}
fn runtime_get(c: &Core, key: &str) -> Option<String> {
    c.db.lock()
        .unwrap()
        .query_row("SELECT value FROM runtime_kv WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .ok()
}
fn runtime_set(c: &Core, key: &str, value: &str) -> Result<()> {
    c.db.lock()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO runtime_kv(key,value) VALUES (?1,?2)",
            [key, value],
        )
        .map_err(err)?;
    Ok(())
}
async fn maintenance(app: AppHandle) {
    let _ = engine::inspect(&app).await;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let c = core(&app);
        let snapshot = c.snapshot();
        if snapshot.operation.is_some()
            || snapshot
                .scan
                .as_ref()
                .is_some_and(|s| s.status == "running")
        {
            continue;
        }
        let now = chrono::Local::now();
        if snapshot.settings.auto_update && snapshot.engine.version.is_some() {
            let due = runtime_get(&c, "last_update_attempt")
                .and_then(|s| s.parse::<i64>().ok())
                .is_none_or(|t| now.timestamp() - t >= 6 * 3600);
            if due && runtime_set(&c, "last_update_attempt", &now.timestamp().to_string()).is_ok() {
                let _ = engine::update(app.clone()).await;
                continue;
            }
        }
        use chrono::Timelike;
        let date = now.format("%Y-%m-%d").to_string();
        if snapshot.settings.schedule_enabled
            && now.hour() == snapshot.settings.schedule_hour
            && runtime_get(&c, "last_scheduled_scan").as_deref() != Some(&date)
            && snapshot.engine.definitions
            && snapshot.engine.version.is_some()
        {
            // Claim first so a restart never duplicates a scheduled scan in the same hour.
            if runtime_set(&c, "last_scheduled_scan", &date).is_ok() {
                if let Err(e) = scanner::start(app.clone(), "quick".into(), None).await {
                    c.log(&app, "error", e);
                }
            }
        }
    }
}
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            let root = std::env::var_os("CLAMAV_DESKTOP_DEV_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(root);
            app.manage(Core::new(root).map_err(std::io::Error::other)?);
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(maintenance(handle));
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "activity" && matches!(event, tauri::WindowEvent::Destroyed) {
                let c = window.state::<Core>();
                c.document.lock().unwrap().activity_floating = false;
                c.publish(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_download_cache,
            cleanup_download_cache,
            get_quarantine_detail,
            verify_quarantine_integrity,
            stop_integrity_check,
            export_quarantine,
            refresh_engine,
            prepare_installer,
            open_installer,
            update_definitions,
            start_scan,
            stop_scan,
            choose_folder,
            quarantine_file,
            restore_file,
            recover_file_transfers,
            save_settings,
            set_activity_panel,
            export_history,
            open_privacy_settings
        ])
        .run(tauri::generate_context!())
        .expect("unable to launch ClamAV Desktop");
}
