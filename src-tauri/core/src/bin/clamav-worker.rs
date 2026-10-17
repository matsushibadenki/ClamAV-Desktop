//! Explicit standalone entry point for the shared headless worker.
#[tokio::main(worker_threads = 2)]
async fn main() {
    if let Err(reason) = clamav_desktop_core::worker::run(std::env::args_os().skip(1).collect()).await {
        eprintln!("{reason}");
        std::process::exit(1);
    }
}
