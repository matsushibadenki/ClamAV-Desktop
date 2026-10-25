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

- Hosted CI started from commit `27b9be4` on 2026-10-14: checkout, frontend dependency installation, updater-key presence check, temporary signing-keychain import, and Apple notarization-profile setup passed. Run: https://github.com/matsushibadenki/ClamAV-Desktop/actions/runs/38504789164 . The build/notarization and Draft Release jobs subsequently completed successfully.

- Downloaded CI artifacts matched SHA-256; the updater signature, signed version and tamper rejection passed locally. The DMG notarization ticket validated. GitHub normalized spaces in the uploaded DMG name; the Draft asset name and public metadata were synchronized to the stable hyphenated name without changing binary contents. Draft remains unpublished.

- Deployment: v0.2.0 was published on explicit user request. Unauthenticated latest-release feed retrieval succeeded; the feed version, archive URL and signature matched the verified release. Public DMG and archive SHA-256 matched release.json. Full installation of a newer version and restart remain unverified.

## Worker settings persistence — 2026-10-16

- Rust workspace: 50 tests passed, 7 environment/fixture-dependent tests ignored. Frontend: 6 tests passed; TypeScript and Vite production build passed.
- Real Unix-socket tests save settings through IPC, reject unsupported language, return the worker-owned snapshot, and retain settings after Core restart. A retry of an earlier save after a later save returns the stored acknowledgement without overwriting the newer settings.
- GUI Backend proxy tests exercise settings writes through worker attachment, confirm that GUI never acquires SQLite ownership, and verify local-mode saves after worker shutdown/reopen.
- Core tests reject invalid language, hour, file-size, retention and cache-limit values. A read-only SQLite failure leaves Document and reopened settings unchanged. Save ordering is serialized by a Core-owned settings gate.
- Socket tests require execution outside the filesystem sandbox (inside it the OS rejects Unix listener creation). User data, release artifacts and GitHub Secrets were not changed.
- Settings controls are unchanged; their existing information-icon help now explains in English/Japanese/Simplified Chinese that manual worker mode stores scheduling choices but does not execute schedules. Native GUI screenshots and hover text layout were not inspected for this change. Full worker scheduling, other operation migration, live ownership transfer and a newer-version updater install remain unfinished.

## Worker definition update IPC — 2026-10-17

- Rust workspace: 52 tests passed, 7 fixture/environment-dependent tests ignored. An additional engine test passed after being added, covering shutdown before launch, operation release, and the persisted cancellation diagnostic. Frontend: 6 tests passed; TypeScript/Vite production build passed.
- A real private Unix socket and harmless `/bin/sleep` child exercised the worker's definition-job lifecycle. While the job was active, snapshot, settings save and scan stop remained responsive; duplicate update requests returned the same recorded busy rejection. Shutdown cancelled and awaited the child, removed the socket, released Core ownership, and retained diagnostics after reopening.
- A controlled stored cooldown rejected update requests before any network access. Clearing the cooldown and replaying the same request still returned its recorded rejection, without launching FreshClam.
- The production engine cancellation-before-launch test records `cancelled`, clears the operation and never runs a download. English/Japanese/Simplified Chinese diagnostic copy provides restart/retry guidance.
- Actual FreshClam/CDN updating through the GUI was not exercised; tests did not access the CDN. Native GUI screenshots, mid-download physical/network interruption, and real service registration remain unverified. No release or user data was changed.

## Worker quarantine details and integrity IPC — 2026-10-17

- Rust workspace: 54 tests passed, 7 fixture/environment-dependent tests ignored. Frontend: 6 tests passed; TypeScript/Vite production build passed.
- Private Unix-socket integration tests verify matching hashes, detail retrieval, duplicate-request suppression after changing the fixture, and immediate busy responses during checking. A dedicated 1 GiB sparse fixture checks progress state, explicit stop, no partial digest, unchanged file size, and shutdown-time cancellation. Unknown record failures replay consistently.
- GUI Backend proxy tests read details and await integrity completion without owning SQLite. An injected SQLite trigger failure is returned through the job-result IPC and Tauri adapter rather than being treated as success. Source fixture bytes remain unchanged.
- Completed results are correlated with the original request UUID; the worker keeps a bounded 64-entry memory cache. Persistent integrity records remain in SQLite. Result-cache loss or eviction reports unknown outcome rather than starting another check.
- No native GUI screenshots were taken for this change. Physical-device removal and stalled OS reads remain unverified; shutdown intentionally retains Core ownership while waiting for a blocking read. Published releases and user data were not modified.

## Worker quarantine, restore and transfer recovery — 2026-10-17

- Rust workspace: 56 tests passed, 7 fixture/environment-dependent tests ignored in the default run. The ignored IPC ClamAV test was then run explicitly and passed with an installed engine and a harmless local custom signature. Frontend: 6 tests passed; TypeScript/Vite build passed.
- Real IPC tests recover a committed transfer journal, roll back an uncommitted journal, preserve an occupied restore destination, and retain restored status across Core restart. Replaying an earlier recovery does not process a newly created journal. Replaying a failed restore after removing its conflict still returns the original failure; a new request succeeds.
- The ClamAV fixture exercises scan → quarantine → restore entirely through IPC. After restore, replaying the original quarantine request leaves the restored file intact. No CDN or malware samples are used.
- A 32 MiB sparse restore fixture verifies control requests remain responsive and shutdown waits for the accepted restore, final hash and persisted restored record. GUI Backend proxy tests propagate a missing-record restore failure and perform recovery without opening SQLite.
- Shared Core operation stages are translated in English/Japanese/Simplified Chinese. The file gate now rejects queued operations as busy, and scan/other-operation conflicts are checked before either local or worker file jobs begin.
- Native screenshots, real cross-volume IPC transfers, physical unplugging and exFAT hardware behavior were not tested for this change. Existing transfer-journal tests still cover copying/rollback on controlled filesystem fixtures. No public release or user data was changed.

## Worker exports and cache management — 2026-10-17

- Rust workspace (serial execution): 58 tests passed, 7 fixture/environment-dependent tests ignored. Frontend: 6 tests passed; TypeScript/Vite production build passed.
- IPC tests export history JSON exceeding the 8 MiB response limit directly to disk, parse it, and verify 0600 permissions. Quarantine/diagnostics retain schemaVersion=1 and diagnostic appUpdate metadata. Replaying an export does not overwrite later user-written content.
- Exports reject application-data destinations, detected original files, symlink destinations, relative paths and oversized metadata; original bytes remain intact. History export now shares the same protected atomic writer as other exports.
- Cache tests create controlled aged attempts, retain the prepared installer and unknown file, remove only eligible attempts, and show that replaying cleanup leaves a newly added aged attempt intact. A fresh cleanup request removes it. GUI Backend proxy tests exercise cache reads, cleanup and diagnostic export without owning SQLite.
- Native save-dialog screenshots and rendering of the export stage were not inspected for this change. Shutdown/job handling is shared with the already-tested file-job lifecycle. No published release, network service or user data was modified.

- Verification caveat: a parallel workspace run intermittently failed the existing diagnostics Core-reopen test with `data_in_use`; that test and the full workspace passed with `--test-threads=1`. This was investigated and fixed in the follow-up section below using a deterministic fork-inheritance regression test.

## Engine inspection IPC and fork-inherited ownership — 2026-10-17

- Added a deterministic fork test: a child retains the inherited owner descriptor while the parent destroys Ownership and tries to reacquire. Before the fix it failed with data_in_use; after explicit acquiring-PID LOCK_UN it passed. A separate inherited-PID guard test proves it cannot unlock a live acquiring owner. CLOEXEC, lock-inode identity and active-owner exclusion remain checked.
- The mechanism matches [Apple flock(2)](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/flock.2.html): fork/dup descriptors refer to the same lock, so closing only the parent's descriptor does not necessarily release it. This reproduces the normal-drop/reopen failure without changing the existing diagnostic test or adding sleeps/retries to acquisition.
- Engine IPC tests execute the real local engine probe, retrieve Engine metadata/diagnostics, and prove a replay does not probe again. A controlled pending probe tests snapshots, scan-stop responses, busy/replayed rejection, failure results, shutdown waiting and retained ownership. GUI Backend proxy rechecks the engine without opening the DB.
- Frontend: 6 tests passed; TypeScript/Vite production build passed. No native screenshots, newly published release, user-data mutation or network/CDN update was performed. Live GUI ownership transfer remains unfinished.

- Final Rust verification: 62 workspace tests passed, 7 fixture/environment-dependent tests ignored. Three full parallel runs passed: default parallelism, 16 test threads, then default parallelism again. This includes the previously intermittent diagnostics reopen test, separate-process ownership exclusion/crash recovery, fork regression, engine IPC and GUI proxy checks.

## Live GUI / worker handoff — 2026-10-17

- Rust workspace: 69 tests passed, 7 existing fixture/environment tests ignored. Commands: `cargo build --manifest-path src-tauri/Cargo.toml --workspace --bins --locked`, then `cargo test --manifest-path src-tauri/Cargo.toml --workspace --locked`. The new integration target ensures the real GUI executable is built for clean test runs as well.
- Actual application executable `--worker` subprocesses verify local → worker → local, saved settings, native panel placement, exclusive ownership, proxy disposal without worker shutdown, reattachment and graceful return. Invalid/relative worker arguments exit before Tauri initialization. No implicit debug-data-root fallback is accepted by the worker.
- Tests cover outstanding Backend/Core references, active operation and prepared-installer rejection, spawn failure and early exit with local recovery, and an unreachable owner that prevents recovery until its OS lock is released. The atomic idle-shutdown IPC test proves active work is not cancelled and a replay retains the original busy result.
- Standard sandbox execution cannot create a Unix socket on this host (`Operation not permitted`). The complete suite passed outside that sandbox with dedicated `/private/tmp` fixtures; the initial two subprocess failures were due to this environment restriction.
- Frontend: 6 tests passed; TypeScript/Vite production build passed. Browser plugin skill was unavailable, so cached Playwright 1.60.0 and Chromium were used at `http://127.0.0.1:1432/` with 1180×800 and 720×580 viewports in Japanese, English and Simplified Chinese. App identity/content, absence of error overlays, language switching, help activation, disabled preview-only native controls and zero horizontal overflow passed. Console warnings/errors and page errors were empty. Screenshots are in `/private/tmp/clamav-handoff-{ja,en,zh-CN}-{1180,720}.png`.
- Native QA uses a temporary `.app` containing the development binary and a dedicated data directory. Via computer-use accessibility controls, Settings → worker → GUI succeeded twice. The worker used the detected local engine and definitions. A delayed engine refresh on return was found, fixed, rebuilt and verified: returning to GUI now immediately retains a usable engine display. Screenshot: `/private/tmp/clamav-handoff-native-worker.png`. Native console instrumentation was not performed. No actual scans, CDN requests or installer launches were needed for these UI checks.
- The application executable now hosts the shared worker entry, so no separate helper needs packaging. This development QA bundle does not establish Developer ID/notarization, login service behavior, helper file permissions, Intel/macOS 12 support or signed-update replacement. Those remain Pending; LaunchAgent and worker scheduling remain unfinished. Session activity logs reset with owner changes; saved records and layout are retained.


## Shared GUI / worker scheduling — 2026-10-25

- Rust workspace: 81 tests passed, 8 fixture/environment-dependent tests ignored in the default suite. The new ignored actual-worker scheduled scan test was also run explicitly and passed. Frontend: 6 tests passed; TypeScript/Vite production build and Rust workspace check passed. Rust commands used `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0` with `--locked --offline`, leaving the target directory at approximately 2.2 GiB rather than restoring the previous large debug/incremental cache.
- Ten injected-clock scheduler tests cover one claim per local day, settings/clock changes, startup recovery after reopening Core, legacy claim import, disabled/re-enabled pending jobs, one-hour expiry without catch-up, missing prerequisites, scan priority over updates, DST skipped/repeated hours, and a clock rollback/timezone change between claim and launch. Scan history and scheduled results commit together; an injected SQLite failure rolls both back. An update-attempt save failure releases the operation without launching a child.
- Worker lifecycle coverage uses a controlled harmless `/bin/sleep` child for automatic updating. An active job rejects idle shutdown; ordinary shutdown cancels and reaps the owned child; maintenance cannot launch an update after stopping. No FreshClam CDN request was made.
- The actual worker binary scanned one harmless text file in a dedicated temporary HOME/Downloads using the installed ClamAV engine and existing definitions. It completed without any GUI, persisted the completed daily record and one-file history, shut down cleanly, and retained them after Core reopen. Fixture bytes remained unchanged and quarantine stayed empty. This verifies an already-running worker, not OS login registration or real sleep/wake behavior.
- Socket/process tests ran outside the filesystem sandbox because this host denies Unix listener creation inside it. All integration data used dedicated temporary directories. User scan targets and production app data were not used.
- Browser plugin skill was unavailable; cached Playwright 1.60.0/Chromium tested the Vite app at `http://127.0.0.1:1432/`, title `ClamAV Desktop`. Tauri IPC was explicitly mocked with isolated fixture state. Settings navigation, language switching, pending/missed/completed rendering, 1180×800 and 720×580 layouts, and absence of horizontal overflow passed for Japanese, English and Simplified Chinese. Browser warnings/errors and page errors were empty. Screenshots: `/private/tmp/clamav-scheduler-{ja,en,zh-CN}-{1180,720}.png`; Japanese narrow, English wide and Chinese narrow images were visually inspected. These browser checks do not establish native WebView-to-worker integration.
- LaunchAgent registration/permission UI is Next. Actual login, reboot, sleep/wake, helper access permissions, macOS 12/Intel, and the next formally signed/notarized release remain Pending. No OS service was registered or public release modified.


## Published upgrade exercise and fixes — 2026-10-25

- Prepared and published signed/notarized v0.3.0 from commit `aad2e33c36351fb7555319c543cfa7d5b6d2cb11` with explicit user approval. App notarization: `3d567b35-9153-4902-a2c6-0e4f081ec4de`; DMG: `eab7e31b-632c-45ba-90ff-74deadbfe499`. All six GitHub asset hashes matched local files; the unauthenticated public feed matched byte-for-byte. Archive SHA-256: `85f7676a645cc6050f69fcca3fcd4c947c7728352f2c4505e5e0324a2e41b8ff`; DMG: `cc8ef3ea9a2c4280bc96cc250bdbcc9a65a0c3c1e19f59bb365e2e0eb8bfd036`.
- The untouched public 0.2.0 archive matched GitHub's hash, and the extracted app passed code-signature and stapled-ticket validation. Its actual native UI detected 0.3.0, but the install button did not display the JavaScript confirmation or begin downloading. This is a failed upgrade from the untouched public client, not a completed end-to-end update.
- Replaced `window.confirm` with the existing app Modal. A local ad-hoc QA build advertised 0.2.0 using a Tauri build-time version override; it retained the real public feed, public key, signed-version verification, download and install code. This is a modified QA client, not the published 0.2.0 binary. Native Japanese/English/Simplified Chinese confirmation text and cancellation were checked. Screenshots are under `/private/tmp/clamav-upgrade-0.2-to-0.3/fixed-confirm*.png`.
- The repaired QA client downloaded the actual public 0.3.0 archive but failed to unpack `._ClamAV Desktop.app` before replacement. Raw Python tarfile inspection reproduced the root AppleDouble entry; macOS tar listing hid it. v0.3.1 packaging disables copyfile metadata and validates raw paths and required app files. The new validator rejects the actual published 0.3.0 archive; a prepared 0.3.1 archive has nine entries and zero AppleDouble entries.
- v0.3.1 preparation: Rust workspace 81 passed / 8 ignored, frontend 6 passed, release utilities 3 passed, TypeScript/Vite build and local Developer ID verification passed. App ZIP preparation passed; the subsequent approved notarization/publication and final update results are recorded below.
- Before native QA, a private SQLite snapshot was saved. The test app used the existing data root with automatic updates and scheduled scans off. Scan, quarantine and transfer records remain byte-for-byte unchanged. After language-switch testing, Japanese and all original setting values were retained; the existing default retention/cache fields (30 days / 1024 MiB) were materialized on save. `/Applications/ClamAV Desktop.app` was not replaced. Test bundles live only in a dedicated temporary directory.


### Successful corrected update to 0.3.1

- Published v0.3.1 from `d03aa5df80ca90d0d08c548d6f0bfcb3e1821362` after explicit approval for the corrected release. App notarization: `7117af3a-f577-4922-a734-05a9f898874e`; DMG: `75223b9d-4b11-4901-b3da-128be39ec23f`. The final notarized archive contains ten normal entries, including the stapled ticket, and no AppleDouble entries. Extracted code signature, stapled ticket and Gatekeeper assessment passed.
- Final SHA-256: updater archive `cbb08ecd0d94818f45ebdfbbbf65d0704dd3c755b328c1e064d578cf8f078d83`; DMG `d1bb0166b666403cef940993f7723793bf11b6e2237a7f46f480c19a9cfbc396`. All six uploaded asset hashes and the release source commit matched before publication. Unauthenticated latest-feed bytes matched the verified local feed.
- On macOS 27.0.1 (26A434), arm64, the repaired native QA client (display version 0.2.0, with current shared core and the Modal fix) detected public 0.3.1. Explicit confirmation started the real HTTPS download, mandatory signature/signed-version verification and Tauri installation. The UI reached “Restart to finish.” All installed bundle files matched the verified distribution bundle byte-for-byte; the installed binary SHA-256 was `3153304881a0762874307023bbe002f565b77c11d9cb2b8792d0a15f4e719c77`.
- Clicking the application's own restart button replaced PID 85589 with PID 98453. A process-only check observed this before any UI tool reattachment, without launching an app. The new native process displayed 0.3.1, a ready engine, and “You are up to date” after another public-feed check. Screenshot: [restarted 0.3.1](images/update-0.3.1-restarted.png).
- All two scan rows, the one quarantine row and zero transfer rows matched the pre-test snapshot exactly. Every original setting value remained unchanged; the default retention/cache fields were added as noted above. No scan, quarantine/restore, CDN definition update, service registration or replacement of `/Applications/ClamAV Desktop.app` was performed.
- macOS code-signature verification inside the filesystem sandbox returned a misleading invalid-signature result for both the untouched distribution and its identical installed copy. The same checks outside the sandbox passed, together with stapled-ticket and Gatekeeper validation. This is distinct from the confirmed UI/AppleDouble defects.
- This completes the repaired-client update/restart path, not an upgrade from the untouched public 0.2.0 or 0.3.0 UI. Those versions need one manual DMG installation to obtain the confirmation fix. Full formally signed old-client → future-release verification after that bootstrap, Intel/macOS 12 and privileged installation into a non-writable destination remain untested.
