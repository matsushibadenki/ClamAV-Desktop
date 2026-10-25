mod app_updater;
use clamav_desktop_core::{
    details, diagnostics, downloads, engine, model, process, quarantine, scanner, store,
};
mod bridge;
mod runtime;
use bridge::{backend, core, Backend};
use clamav_desktop_core::ipc::{Command as WorkerCommand, Payload};
mod installer;
mod renderer;
use model::*;
use std::sync::atomic::Ordering;
use store::*;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
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
    backend(&app)?.snapshot().await
}
#[tauri::command]
fn get_backend_status(app: AppHandle) -> runtime::Status {
    app.state::<runtime::Runtime>().status()
}
#[tauri::command]
async fn set_worker_mode(
    app: AppHandle,
    window: WebviewWindow,
    worker: bool,
) -> Result<runtime::Status> {
    main_window(&window)?;
    // An owned task completes handoff even if the invoking WebView disappears.
    tauri::async_runtime::spawn(async move {
        let executable = std::env::current_exe().map_err(err)?;
        let result = app
            .state::<runtime::Runtime>()
            .switch(worker, executable, true)
            .await;
        // A recreated local Core starts without engine metadata. Refresh now,
        // rather than displaying a missing engine until the next maintenance tick.
        if let Ok(c) = core(&app) {
            if !c.snapshot().engine.checked {
                let _ = engine::refresh(&c).await;
            }
        }
        let _ = app.emit("core:backend", app.state::<runtime::Runtime>().status());
        if let Ok(backend) = backend(&app) {
            backend.publish_cached(&app);
        }
        result
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn refresh_engine(app: AppHandle, window: WebviewWindow) -> Result<Engine> {
    main_window(&window)?;
    backend(&app)?.refresh_engine().await
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
    match backend(&app)?.as_ref() {
        Backend::Local(c) => engine::update(c).await,
        Backend::Worker { client, .. } => {
            match client.request(WorkerCommand::UpdateDefinitions).await? {
                Payload::Ack => Ok(()),
                _ => Err("ipc_invalid_response".into()),
            }
        }
    }
}
#[tauri::command]
async fn start_scan(
    app: AppHandle,
    window: WebviewWindow,
    kind: String,
    target: Option<String>,
) -> Result<String> {
    main_window(&window)?;
    match backend(&app)?.as_ref() {
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
    match backend(&app)?.as_ref() {
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
    backend(&app)?
        .file_operation(quarantine::FileAction::Quarantine(id))
        .await
}
#[tauri::command]
async fn restore_file(app: AppHandle, window: WebviewWindow, id: String) -> Result<()> {
    main_window(&window)?;
    backend(&app)?
        .file_operation(quarantine::FileAction::Restore(id))
        .await
}
#[tauri::command]
async fn recover_file_transfers(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    backend(&app)?
        .file_operation(quarantine::FileAction::Recover)
        .await
}
#[tauri::command]
async fn get_download_cache(app: AppHandle) -> Result<downloads::CacheReport> {
    backend(&app)?.download_cache().await
}
#[tauri::command]
async fn cleanup_download_cache(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<downloads::CacheReport> {
    main_window(&window)?;
    backend(&app)?
        .file_operation(quarantine::FileAction::CleanupCache)
        .await?;
    backend(&app)?.download_cache().await
}
#[tauri::command]
async fn get_quarantine_detail(app: AppHandle, id: String) -> Result<details::Detail> {
    backend(&app)?.quarantine_detail(id).await
}
#[tauri::command]
async fn verify_quarantine_integrity(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<details::Detail> {
    main_window(&window)?;
    backend(&app)?.verify_integrity(id).await
}
#[tauri::command]
async fn stop_integrity_check(app: AppHandle, window: WebviewWindow) -> Result<()> {
    main_window(&window)?;
    backend(&app)?.stop_integrity().await
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
    let destination = file.path().to_str().ok_or("invalid_path")?.to_string();
    backend(&app)?
        .file_operation(quarantine::FileAction::Export {
            kind: clamav_desktop_core::exports::Kind::Quarantine,
            destination: destination.clone(),
            app_update: None,
        })
        .await?;
    Ok(Some(destination))
}

#[tauri::command]
async fn export_diagnostics(app: AppHandle, window: WebviewWindow) -> Result<Option<String>> {
    main_window(&window)?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_file_name("clamav-diagnostics.json")
        .add_filter("JSON", &["json"])
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let destination = file.path().to_str().ok_or("invalid_path")?.to_string();
    backend(&app)?
        .file_operation(quarantine::FileAction::Export {
            kind: clamav_desktop_core::exports::Kind::Diagnostics,
            destination: destination.clone(),
            app_update: Some(
                serde_json::to_value(app.state::<app_updater::Service>().snapshot())
                    .map_err(err)?,
            ),
        })
        .await?;
    Ok(Some(destination))
}

#[tauri::command]
async fn save_settings(app: AppHandle, window: WebviewWindow, settings: Settings) -> Result<()> {
    main_window(&window)?;
    backend(&app)?.save_settings(settings).await
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
    let destination = file.path().to_str().ok_or("invalid_path")?.to_string();
    backend(&app)?
        .file_operation(quarantine::FileAction::Export {
            kind: clamav_desktop_core::exports::Kind::History,
            destination: destination.clone(),
            app_update: None,
        })
        .await?;
    Ok(Some(destination))
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
async fn maintenance(app: AppHandle) {
    if let Ok(c) = core(&app) {
        let _ = engine::refresh(&c).await;
    }
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        // Never retain a Core while sleeping; handoff must be able to drop it.
        let Ok(c) = core(&app) else {
            continue;
        };
        if !c.snapshot().engine.checked {
            let _ = engine::refresh(&c).await;
        }
        if let Err(reason) =
            clamav_desktop_core::scheduler::tick(&c, &clamav_desktop_core::scheduler::SystemClock)
                .await
        {
            if reason != "busy" {
                c.log("error", format!("scheduler: {reason}"));
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
            let state = tauri::async_runtime::block_on(runtime::Runtime::open(
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
            app.manage(state);
            tauri::async_runtime::spawn(bridge::poll_worker(app.handle().clone()));
            tauri::async_runtime::spawn(maintenance(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if window.state::<runtime::Runtime>().status().transitioning { api.prevent_close(); }
                    if window.try_state::<app_updater::Service>().is_some_and(|s| matches!(s.snapshot().phase.as_str(), "downloading" | "installing")) { api.prevent_close(); }
                }
            }
            if window.label() == "activity" && matches!(event, tauri::WindowEvent::Destroyed) {
                if let Ok(b) = backend(window.app_handle()) {
                    let _ = b.set_panel(false, None);
                    b.publish_cached(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_update, check_app_update, install_app_update, restart_app_update,
            get_snapshot, get_backend_status, set_worker_mode,
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
                if app.state::<runtime::Runtime>().status().transitioning { api.prevent_exit(); }
                if app.try_state::<app_updater::Service>().is_some_and(|s| matches!(s.snapshot().phase.as_str(), "downloading" | "installing")) { api.prevent_exit(); }
            }
        });
}
