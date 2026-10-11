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
- Private quarantine destination permissions remain mandatory. exFAT source staging and explicitly requested restore destinations allow non-POSIX permissions; copy verification precedes source staging. Capacity checks reserve 1 MiB and translate ENOSPC into localized feedback.
- Added automatic tests for capacity rejection and an injected late ENOSPC failure, verifying original preservation, temporary cleanup and journal cleanup. This injection tests rollback after copying; it does not prove hardware behavior during a partial write.
- Actual exFAT round-trip and real volume-capacity tests are available as ignored tests via CLAMAV_TEST_VOLUME. They remain pending: hdiutil/diskutil image creation fails with OS errors in this environment. Physical-device removal and exFAT directory-sync durability remain unverified.
- Test volume and temporary app state were removed after verification. The final optimized app uses the normal application-data directory.

## Download storage and quarantine detail verification

- Rust cache test covers the 30-day age policy, dry-run bytes, deletion, prepared-package protection, unknown-file preservation and symlink exclusion. Metadata tests cover file bytes, absent paths and rejected links, including a symlink in an ancestor directory.
- Native debug app uses an isolated `/private/tmp/clamav-storage-qa` fixture. Settings rendered 4 MiB / 2 attempts, with 2 MiB / 1 attempt reclaimable. The delete confirmation and cancellation were exercised. Actual deletion is verified by Rust tests; user downloads were not deleted.
- Quarantine rendered a recovery-pending journal, original/stored/staged/temporary paths, stored SHA-256 and file presence. Detail scrolling, refresh, close and the restored-record filter were exercised with harmless fixtures. Native screenshots showed no clipping or framework overlay at 1180 × 800.
- Native WebView console logs were not captured. Narrow window and additional language UI screenshots remain unverified; translation completeness is checked automatically.

## Configurable cache policy and quarantine filters

- Rust: 22 tests passed, 4 environment-dependent tests ignored. New tests cover legacy settings defaults, saved retention changing eligible attempts, and quota rejection while existing downloads remain intact.
- Frontend: 3 tests passed. Search tests cover Unicode normalization, multiple query terms, translated status, completion filtering, attention/date ordering and preservation of the shared snapshot array.
- Native release app at tauri://localhost: existing settings loaded with 30-day/1024-MiB defaults; new numeric fields and policy explanation rendered. Japanese/English search input, no-results feedback, clearing search and selecting attention order were exercised. Screenshots confirmed readable layout at 1180 × 800. Setting changes in the user's normal data were not saved; persistence is covered by the Rust settings serialization and existing SQLite settings path.
- Release app, ad-hoc signing and DMG integrity checks passed. Narrow-window rendering and native console logs were not inspected for this change.

## Manual integrity and recovery diagnostics

- Rust tests cover match, mismatch, absent file and symbolic-link rejection without altering the source. Existing conflict-recovery test also checks persisted failure text, attempt timestamp and owner-scoped detail retrieval.
- Native debug UI used an isolated harmless `/private/tmp/clamav-integrity-qa` fixture. SHA-256 confirmation rendered a matching result, then a mismatch after changing only the fixture. Both results were read back from SQLite. The changed fixture remained intact.
- Missing-volume recovery failure rendered an expandable OS diagnostic. Retrying from details updated its persisted attempt timestamp while preserving the file. Native screenshots at 1180 × 800 confirmed readable hashing results and scrolling diagnostics.
- Verification is a timestamped hash comparison, not a malware rescan. Long checks currently run to completion; cancellation, live byte progress, narrow-window QA and native console capture remain outside this change.
