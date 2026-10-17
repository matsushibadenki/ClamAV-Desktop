//! Compiling this integration target also builds the real application binary
//! used by the live-handoff unit tests (including on a clean cargo test run).
#[test]
fn application_worker_entry_requires_explicit_absolute_root_without_opening_gui() {
    for args in [
        vec!["--worker"],
        vec!["--worker", "serve", "--data-dir", "relative"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_clamav-desktop"))
            .args(&args)
            .env(
                "CLAMAV_DESKTOP_DEV_DATA_DIR",
                "/private/tmp/must-not-use-worker-env",
            )
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("Usage:") || error.contains("absolute_data_directory_required"));
    }
}
