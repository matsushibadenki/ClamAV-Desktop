//! Explicitly launched worker. No OS service registration or implicit data-root override.
use crate::{
    engine,
    ipc::{Client, Request, Server, REQUEST_LIMIT},
    store::{err, Core, Result},
};
use std::{io::Read, path::PathBuf, sync::Arc};

pub async fn run(args: Vec<std::ffi::OsString>) -> Result<()> {
    if args.len() != 3 || args[1] != "--data-dir" {
        return Err("Usage: clamav-worker serve|request --data-dir ABSOLUTE_PATH".into());
    }
    let root = PathBuf::from(&args[2]);
    if !root.is_absolute() {
        return Err("absolute_data_directory_required".into());
    }
    match args[0].to_str() {
        Some("serve") => {
            let core = Arc::new(Core::new(root)?);
            let server = Server::bind(core.clone())?;
            eprintln!("worker_ready: {}", server.path().display());
            let inspection = tokio::spawn(async move {
                let _ = engine::inspect(&core).await;
            });
            let result = server
                .run(async {
                    let mut term =
                        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                            .expect("SIGTERM handler");
                    tokio::select! { _ = tokio::signal::ctrl_c() => (), _ = term.recv() => () }
                })
                .await;
            // Release the inspection's Core reference before exiting.
            inspection.abort();
            let _ = inspection.await;
            result
        }
        Some("request") => {
            let mut bytes = vec![];
            std::io::stdin()
                .take(REQUEST_LIMIT as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(err)?;
            if bytes.len() > REQUEST_LIMIT {
                return Err("ipc_frame_too_large".into());
            }
            let request: Request =
                serde_json::from_slice(&bytes).map_err(|_| "ipc_invalid_request")?;
            let response = Client::new(root)?.send(&request).await?;
            println!("{}", serde_json::to_string(&response).map_err(err)?);
            if let Some(reason) = response.error {
                return Err(reason);
            }
            Ok(())
        }
        _ => Err("invalid_worker_mode".into()),
    }
}
