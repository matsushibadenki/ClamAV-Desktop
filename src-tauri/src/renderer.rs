//! Coordinates native views of the Rust-owned document. No window owns domain state.
//! This UI has no high-load graphics; GPU/frame transport is intentionally unnecessary.
use crate::bridge::backend;
use crate::store::*;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
pub struct Renderer;
impl Renderer {
    pub fn activity_panel(app: &AppHandle, floating: bool, side: Option<String>) -> Result<()> {
        if let Some(side) = &side {
            if side != "left" && side != "right" {
                return Err("invalid_dock".into());
            }
            // Applied to the shared Rust Renderer state after native window handling.
        }
        if floating {
            if let Some(window) = app.get_webview_window("activity") {
                window.set_focus().map_err(err)?;
            } else {
                WebviewWindowBuilder::new(
                    app,
                    "activity",
                    WebviewUrl::App("index.html?panel=activity".into()),
                )
                .title("ClamAV Desktop · Activity")
                .inner_size(440.0, 560.0)
                .min_inner_size(320.0, 360.0)
                .build()
                .map_err(err)?;
            }
        } else if let Some(window) = app.get_webview_window("activity") {
            window.close().map_err(err)?;
        }
        let b = backend(app);
        b.set_panel(floating, side)?;
        b.publish_cached(app);
        Ok(())
    }
}
