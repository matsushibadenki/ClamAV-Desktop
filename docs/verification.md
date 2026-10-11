# Verification — 2026-10-11

- `npm run build`: TypeScript and Vite production build passed.
- `npm test`: 1 localization/date contract test passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --offline`: 8 tests passed, 2 fixture-dependent tests skipped by default.
- Both fixture-dependent tests were run explicitly and passed: a real ClamAV process scanning clean/detected custom-signature fixtures, and official PKG PGP/Apple-signature/Gatekeeper verification. No engine was installed by these tests.
- Native macOS UI: custom folder scan returned 2 scanned files / 1 EICAR test detection / 0 errors. Quarantine removed the original test file; restore returned the same 68-byte file. Existing destinations are protected by the exclusive-move unit test.
- Native FreshClam updated definitions in the app's own database directory, including an application-data path containing spaces. Homebrew's database was not modified.
- Native official installer download completed and displayed the verified state. Opening/installing the PKG was intentionally left to the normal user workflow because a working engine already exists.
- Native activity panel floated into a second Tauri window and docked back. Scan history remained after restarting the final build.
- Browser QA: English/Japanese/Simplified Chinese, 320/375/414/768/1440px widths, five pages, settings interactions and installer dialog. Checked widths had no horizontal overflow; console contained no warnings or errors.
- Final development `.app` bundle built and launched successfully. No Developer ID distribution signature or notarization is claimed. Intel, macOS 12, a clean engine installation, and app-closed background scheduling are not verified.

Temporary test files and fixture downloads were removed. App-generated history and the verified package cached by the app remain in its normal application-data directory. Delivery screenshots and the design concept remain in `docs/`.
