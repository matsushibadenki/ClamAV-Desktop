#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // The signed application executable also hosts the headless worker. Branch
    // before Tauri initialization: no WebView, GUI state, or implicit data root.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--worker")) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("worker runtime");
        if let Err(reason) = runtime.block_on(clamav_desktop_core::worker::run(
            std::env::args_os().skip(2).collect(),
        )) {
            eprintln!("{reason}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(target_os = "macos")]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--login-service-check"))
    {
        if std::env::args_os().len() != 2 {
            eprintln!("Usage: clamav-desktop --login-service-check");
            std::process::exit(1);
        }
        match clamav_desktop_core::login_service::current_user_preflight() {
            Ok(plan) => println!(
                "{}",
                serde_json::to_string_pretty(&plan).expect("serializable service plan")
            ),
            Err(reason) => {
                eprintln!("{reason}");
                std::process::exit(1);
            }
        }
        return;
    }
    clamav_desktop_lib::run();
}
