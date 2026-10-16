mod app_updater;
use clamav_desktop_core::{
    details, diagnostics, downloads, engine, model, process, quarantine, scanner, store,
};
mod bridge;
use bridge::{backend, core, Backend};
use clamav_desktop_core::ipc::{Command as WorkerCommand, Payload};
mod installer;
mod renderer;
use model::*;
use std::sync::atomic::Ordering;
use store::*;
use tauri::{AppHandle, Manager, WebviewWindow};
fn main_window(window: &WebviewWindow) -> Result<()> {
    if window.label() != "main" {
        return Err("main_window_required".into());
    }
    if window
        .app_handle()
        .state::<app_updater::Service>()
        .blocks_operations()
    {
        return Err("busy".into());
    }
    Ok(())
}
#[tauri::command]
fn get_app_update(app: AppHandle) -> app_updater::Status {
    app.state::<app_updater::Service>().snapshot()
}
#[tauri::command]
async fn check_app_update(app: AppHandle, window: WebviewWindow) -> Result<app_updater::Status> {
    main_window(&window)?;
    app_updater::check(app).await
}
#[tauri::command]
async fn install_app_update(app: AppHandle, window: WebviewWindow) -> Result<app_updater::Status> {
    main_window(&window)?;
    app_updater::install(app).await
}
#[tauri::command]
fn restart_app_update(app: AppHandle, window: WebviewWindow) -> Result<()> {
    if window.label() != "main" {
        return Err("main_window_required".into());
    }
    app_updater::restart(&app)
}
#[tauri::command]
async fn get_snapshot(app: AppHandle) -> Result<Snapshot> {
    backend(&app).snapshot().await
}
#[tauri::command]
async fn refresh_engine(app: AppHandle) -> Result<Engine> {
    engine::inspect(core(&app)?.as_ref()).await
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
    engine::update(core(&app)?.as_ref()).await
}
#[tauri::command]
async fn start_scan(
    app: AppHandle,
    window: WebviewWindow,
    kind: String,
    target: Option<String>,
) -> Result<String> {
    main_window(&window)?;
    match backend(&app).inner() {
        Backend::Local(c) => scanner::start(c.clone(), kind, target).await,
        Backend::Worker { client, .. } => match client
            .request(WorkerCommand::StartScan { kind, target })
            .await?
        {
            Payload::ScanStarted { scan_id } => Ok(scan_id),
            _ => Err("ipc_invalid_response".into()),
        },
    }
}
#[tauri::command]
async fn stop_scan(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    match backend(&app).inner() {
        Backend::Local(c) => c.cancel.store(true, Ordering::Relaxed),
        Backend::Worker { client, .. } => {
            client.request(WorkerCommand::StopScan).await?;
        }
    }
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
    tauri::async_runtime::spawn_blocking(move || quarantine::quarantine(core(&app)?.as_ref(), &id))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn restore_file(app: AppHandle, window: WebviewWindow, id: String) -> Result<()> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || quarantine::restore(core(&app)?.as_ref(), &id))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn recover_file_transfers(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let c = core(&app)?;
        let _gate = c.file_gate.lock().unwrap();
        quarantine::refresh_items(&c)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn get_download_cache(app: AppHandle) -> Result<downloads::CacheReport> {
    tauri::async_runtime::spawn_blocking(move || downloads::report(core(&app)?.as_ref()))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn cleanup_download_cache(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<downloads::CacheReport> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || downloads::cleanup(core(&app)?.as_ref()))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn get_quarantine_detail(app: AppHandle, id: String) -> Result<details::Detail> {
    tauri::async_runtime::spawn_blocking(move || details::get(core(&app)?.as_ref(), &id))
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
        details::verify(core(&app)?.as_ref(), &id)?;
        details::get(core(&app)?.as_ref(), &id)
    })
    .await
    .map_err(err)?
}
#[tauri::command]
fn stop_integrity_check(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    let c = core(&app)?;
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
    c.publish();
    Ok(())
}
#[tauri::command]
async fn export_quarantine(app: AppHandle, window: WebviewWindow) -> Result<Option<String>> {
    main_window(&window)?;
    let c = core(&app)?;
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
        let bytes = serde_json::to_vec_pretty(&details::export(&c)?).map_err(err)?;
        details::write_export(&c, &destination, &bytes)?;
        Ok(Some(destination.to_string_lossy().into()))
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn export_diagnostics(app: AppHandle, window: WebviewWindow) -> Result<Option<String>> {
    main_window(&window)?;
    let c = core(&app)?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_file_name("clamav-diagnostics.json")
        .add_filter("JSON", &["json"])
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let destination = file.path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let mut report = serde_json::to_value(diagnostics::report(&c)?).map_err(err)?;
        if let Some(fields) = report.as_object_mut() {
            fields.insert(
                "appUpdate".into(),
                serde_json::to_value(app.state::<app_updater::Service>().snapshot())
                    .map_err(err)?,
            );
        }
        let bytes = serde_json::to_vec_pretty(&report).map_err(err)?;
        details::write_export(&c, &destination, &bytes)?;
        Ok(Some(destination.to_string_lossy().into()))
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn save_settings(app: AppHandle, window: WebviewWindow, settings: Settings) -> Result<()> {
    main_window(&window)?;
    backend(&app).save_settings(settings).await
}

#[tauri::command]
fn set_activity_panel(app: AppHandle, floating: bool, side: Option<String>) -> Result<()> {
    renderer::Renderer::activity_panel(&app, floating, side)
}
#[tauri::command]
async fn export_history(app: AppHandle, window: WebviewWindow) -> Result<Option<String>> {
    main_window(&window)?;
    let c = core(&app)?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_file_name("clamav-history.json")
        .add_filter("JSON", &["json"])
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec_pretty(&c.snapshot().history).map_err(err)?;
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
    let Ok(c) = core(&app) else {
        return;
    };
    let _ = engine::inspect(&c).await;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
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
                let _ = engine::update(&c).await;
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
                if let Err(e) = scanner::start(c.clone(), "quick".into(), None).await {
                    c.log("error", e);
                }
            }
        }
    }
}
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.manage(app_updater::Service::new(app.package_info().version.to_string()));
            let root = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            let root = std::env::var_os("CLAMAV_DESKTOP_DEV_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(root);
            let state = tauri::async_runtime::block_on(Backend::open(
                root,
                std::sync::Arc::new(bridge::TauriNotifier(app.handle().clone())),
            )).map_err(|reason| {
                if reason.starts_with("data_in_use") {
                    rfd::MessageDialog::new()
                        .set_title("ClamAV Desktop")
                        .set_level(rfd::MessageLevel::Warning)
                        .set_description("このデータは別のプロセスが使用中で、workerへ接続できません。使用中のアプリを終了してから再度開いてください。\n\nAnother process owns this data and the worker is unavailable. Close the other app before reopening.\n\n另一个进程正在使用此数据，且无法连接后台进程。请关闭正在使用数据的应用后重新打开。")
                        .show();
                }
                std::io::Error::other(reason)
            })?;
            let remote = matches!(&state, Backend::Worker { .. });
            app.manage(state);
            let handle = app.handle().clone();
            if remote { tauri::async_runtime::spawn(bridge::poll_worker(handle)); }
            else { tauri::async_runtime::spawn(maintenance(handle)); }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if window.try_state::<app_updater::Service>().is_some_and(|s| matches!(s.snapshot().phase.as_str(), "downloading" | "installing")) { api.prevent_close(); }
                }
            }
            if window.label() == "activity" && matches!(event, tauri::WindowEvent::Destroyed) {
                let b = backend(window.app_handle());
                let _ = b.set_panel(false, None);
                b.publish_cached(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_update, check_app_update, install_app_update, restart_app_update,
            get_snapshot,
            get_download_cache,
            cleanup_download_cache,
            get_quarantine_detail,
            verify_quarantine_integrity,
            stop_integrity_check,
            export_quarantine,
            export_diagnostics,
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
        .build(tauri::generate_context!())
        .expect("unable to launch ClamAV Desktop")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if app.try_state::<app_updater::Service>().is_some_and(|s| matches!(s.snapshot().phase.as_str(), "downloading" | "installing")) { api.prevent_exit(); }
            }
        });
}
