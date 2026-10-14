# Verification — 2026-10-11

## Worker and local IPC — 2026-10-13

- Final workspace suite: 46 Rust test entries passed; 6 fixture/environment tests skipped by default. The real-ClamAV IPC fixture was explicitly executed and passed with 2 scanned files / 1 harmless custom-signature detection. Retrying the same start request returned the original scan ID and left one history record.
- Real worker subprocess tests cover client disconnection, continued ownership, graceful shutdown and socket removal, SIGKILL followed by stale-socket replacement, and refusal of relative roots/debug environment overrides. All fixtures use dedicated temporary data.
- GUI backend tests verify startup attachment without a second Core, no local fallback when the owner is unreachable, renderer layout in the proxy, worker survival after proxy close, and reopening a local Core after worker shutdown.
- Protocol tests cover injected UID mismatch, version/time/target validation, unknown commands, malformed/partial/oversized frames, a silent-peer timeout, bounded oversized-snapshot replies, private socket mode, unsafe socket preservation, one listener per Core, persistent replay responses, ambiguous claims and a full request ledger. The cross-user rejection branch is tested with injected UID values; an actual different-user connection is still pending.
- Frontend: 6 tests passed, including connection callback forwarding and subscription cleanup when the initial state or either subscription fails. TypeScript/Vite build passed. The new messages exist in English/Japanese/Simplified Chinese.
- Unix socket bind tests require execution outside the standard sandbox; inside it, macOS returned EPERM. No service registration or privileges were added to the product.
- Updated release app/worker and standard development DMG were built; app codesign and image checksum verification passed and SHA-256 metadata was regenerated. The worker binary is a separate developer artifact and is not yet bundled inside the .app. No persistent background service was registered.
- After the Mac was unlocked, native release GUI connected to the actual worker and displayed the existing engine/history. An unsupported engine-refresh action rendered its Japanese message. The activity panel floated and docked again. Stopping the worker rendered the connection-loss toast; restarting it removed the toast automatically. Japanese wording was shortened after screenshot inspection found an awkward sentence-ending wrap at 1180 × 800; the rebuilt app displayed the revised message on one line. The worker was stopped with SIGINT, the socket was removed, and the final GUI was reopened in local mode with successful engine refresh. Additional languages, narrow windows and native console capture remain unverified for this change.

## Shared Rust core and process ownership — 2026-10-13

- Workspace suite: 34 Rust tests passed (including the subprocess fixture entry); 5 environment-dependent tests skipped by default. The new headless scan fixture was then explicitly run and passed, bringing executed application-flow coverage to scan, quarantine, integrity verification, restore and restart using installed ClamAV and a harmless custom signature. Fixture files were removed; user files were not scanned.
- The independent core crate built and ran the headless fixture without Tauri dependencies. Six ownership/notification test entries cover a real second-process rejection, unchanged live scan/journal data, SIGKILL lock release, recovery timestamps remaining unchanged on a second restart, initialization failure release, aliases, unsafe locks and a callback reading Core again.
- Frontend: 3 tests passed; TypeScript/Vite production build passed. Native release app loaded existing state, recognized ClamAV 1.5.4 and rendered the main and floating activity windows. Floating and re-docking confirmed Rust-owned state notifications across both WebViews. Native console capture, narrow-window rendering and the startup-conflict dialog were not inspected for this change.
- Release app built with ad-hoc signing. Standard DMG packaging failed under the sandbox with hdiutil's device-not-configured error. A plain UDZO image was created outside the sandbox from the same generated app; it does not contain the usual Finder layout or Applications shortcut. App codesign verification and DMG checksum verification passed, and SHA-256/release metadata were regenerated. No service registration, formal notarization or publication was performed.

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

## Quarantine export and interruptible integrity checks

- Rust tests cover cancellation after the first 64KiB block, monotonic byte progress, a null partial digest, unchanged file contents, and exports containing both cancelled and restored records. Export tests verify protected original/database destinations and busy file-gate rejection.
- Native debug QA uses an isolated harmless 8GiB sparse file. The UI showed byte/percentage progress and stopped at 66.50MiB, then displayed a cancelled result with no measured hash. The source size remained unchanged.
- Native save dialog exported to a dedicated /private/tmp JSON file. Parsing verified schemaVersion=1 and all 3 records, including a cancelled integrity result, an existing verified result, and a restored record excluded by the UI filter. Output permissions were 0600.
- Verification stops between read operations; OS-blocked disk I/O can delay acknowledgement. The result-saving phase is not interruptible. Narrow-window and console QA remain unverified for this change.

## Engine/update diagnostics and support bundle

- Rust tests verify HTTP/rate-limit, trust, timeout, disk and network classification; latest-attempt persistence across Core restart; success clearing a failure; retry backoff; partial report generation when cache information is inaccessible. A FreshClam outcome test rejects cooldown output even with exit code zero.
- Native debug UI used a dedicated fixture with a stored rate-limit deadline. The definition-update button was disabled and the retry deadline/guidance rendered. This was controlled fixture evidence, not a live CDN rejection.
- Installer preparation was rejected by a real local 257MiB cache policy with a 2MiB fixture, before downloading. The modal and settings showed the reason and retry action; retry updated the persisted attempt.
- Native save dialog exported a report to /private/tmp. Parsing confirmed app version, engine/settings, rate-limit deadline, cache failure, cache bytes and private 0600 permissions. User data and engine installations were not modified.
- Official classification reference: [ClamAV FreshClam FAQ](https://docs.clamav.net/faq/faq-freshclam.html). The 24-hour fallback is this application's conservative backoff when no usable server/tool retry time is present.
- Live HTTP 403/429/503/1020, disconnected-network failures, narrow-window layout and native console capture remain unverified.

- Updated screen copy follows the information-icon rule: functional explanations in Settings, installer and cache controls use localized native hover help with accessible labels; failure reasons and retry deadlines remain visible. Modal error notifications now stay inside the dialog width.

- Browser preview at http://127.0.0.1:1432/ confirmed localized information-icon help and explicit switch names; clicking a help icon did not toggle its setting. Error/warning console capture returned no entries. This preview does not exercise native updater commands.

## Software updater and signed release — 2026-10-14

- Frontend: 6 tests passed. Release manifest helpers: 2 tests passed. Workspace Rust tests passed outside the filesystem sandbox; Unix-socket tests cannot run within that sandbox. Environment-dependent tests remain ignored.
- An additional signed-archive test verified the configured public key, signed version 0.2.0, and rejection after changing archive bytes.
- The final Apple Silicon 0.2.0 app and DMG passed Developer ID verification and stapled notarization-ticket validation. Verified checksums and updater manifests were collected into the ignored release-output directory.
- Native Settings at 1180 × 800 rendered the software updater with current version 0.2.0, help, checking state, and a localized failure/retry message for the GitHub feed before any public release exists. This does not prove a full install/restart cycle.
- GitHub repository Secrets and environments were empty when checked. Hosted CI, a published newer-version upgrade, Intel, macOS 12, narrow-window screenshots, and native console capture remain pending. No source push or GitHub Release was performed.

- 2026-10-14 follow-up: all six required repository Secret names are present. Secret values and the uploaded p12 identity have not been inspected; Hosted CI validation is still outstanding.
