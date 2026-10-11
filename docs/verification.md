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

## Roadmap implementation — 2026-10-11

- Cross-volume transfers now use private staging, synced copies, SHA-256 verification and a SQLite transfer journal. Quarantine re-scans the copy before publication. The source is retained until the owning quarantine row commits.
- Final standard suite: 16 Rust tests passed, 3 fixture-dependent tests skipped by default; 1 frontend localization test passed. The separate-volume integration test was explicitly run and passed on a mounted APFS disk image with a different device ID.
- Failure tests cover verification rejection, changed source bytes, destination collisions, interrupted publication, interrupted rollback, committed cleanup, and recovery while the original path is reoccupied. Recovery preserves files and reports pending work when it cannot safely finish.
- Native UI confirmed 2 files scanned, 1 EICAR detection, successful cross-volume quarantine, then restore to the mounted volume. SQLite ended with status `restored` and zero transfer journals. The restored fixture was 68 bytes. Evidence: `cross-volume-verification.png`.
- `npm run dist:mac` passed: release app 18.73 MiB and arm64 development DMG 6.31 MiB. `codesign --verify --deep --strict` and `hdiutil verify` passed; SHA-256 and release metadata were generated.
- Signed release preflight correctly stopped when APPLE_SIGNING_IDENTITY was unset. Developer ID signing and notarization remain unverified; no submission to Apple or release publication was performed.
- Separate-volume support requires effective private directory permissions and ownership. Unsupported filesystems are rejected before moving the original. Physical-device removal, disk-full behavior and an alternative isolation strategy for exFAT remain next work.
- Test volume and temporary app state were removed after verification. The final optimized app uses the normal application-data directory.
