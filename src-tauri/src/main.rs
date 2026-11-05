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
            Ok(plan) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("status runtime");
                let os = runtime.block_on(clamav_desktop_core::login_service::inspect_os(&plan));
                let mut output = serde_json::to_value(&plan).expect("serializable plan");
                output["permission"] = serde_json::to_value(os.permission).unwrap();
                output["loaded"] = serde_json::to_value(os.loaded).unwrap();
                output["permissionSupported"] = serde_json::json!(os.permission_supported);
                output["osQueryFailed"] = serde_json::json!(os.query_failed);
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            }
            Err(reason) => {
                eprintln!("{reason}");
                std::process::exit(1);
            }
        }
        return;
    }
    #[cfg(target_os = "macos")]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--login-service")) {
        use clamav_desktop_core::login_service::{manage_current, Operation};
        let operation = match (
            std::env::args_os().len(),
            std::env::args_os().nth(2).as_deref(),
        ) {
            (3, Some(value)) if value == "enable" => Operation::Enable,
            (3, Some(value)) if value == "disable" => Operation::Disable,
            (3, Some(value)) if value == "repair" => Operation::Repair,
            (3, Some(value)) if value == "recover" => Operation::Recover,
            _ => {
                eprintln!("Usage: clamav-desktop --login-service enable|disable|repair|recover");
                std::process::exit(1);
            }
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("service runtime");
        if let Err(reason) = runtime.block_on(manage_current(operation)) {
            eprintln!("{reason}");
            std::process::exit(1);
        }
        println!(
            "{}",
            match operation {
                Operation::Enable => "login_service_registered",
                Operation::Disable => "login_service_removed",
                Operation::Repair => "login_service_repaired",
                Operation::Recover => "login_service_recovered",
            }
        );
        return;
    }
    clamav_desktop_lib::run();
}
