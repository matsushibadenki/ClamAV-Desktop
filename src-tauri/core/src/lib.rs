//! Rust-owned application state and operations, independent of Tauri and WebView.
pub mod details;
pub mod diagnostics;
pub mod downloads;
pub mod engine;
pub mod file_transfer;
#[cfg(unix)]
pub mod ipc;
pub mod model;
mod ownership;
pub mod process;
pub mod quarantine;
pub mod scanner;
pub mod store;
