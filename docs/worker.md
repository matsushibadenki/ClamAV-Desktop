# Worker / local IPC

手動起動・GUIからの切替、LaunchAgentの登録・解除、OS許可の表示と復旧に対応。GUIを終了してもworkerは継続する。公開0.3.2の登録には署名判定の不具合があり、修正済み0.3.3は公証・公開済みで、同じコード署名ハッシュの候補で登録・解除・OS許可変更後の復旧を実機確認済み。

## 実装済み

- 🟢 [Done] Tauriに依存しない`clamav-worker`。専用データ領域を明示し、Coreの所有権を取得してからソケットを公開する。
- 🟢 [Done] 状態取得、検査開始、検査停止、設定保存、手動定義更新、worker終了。同じユーザーのUnix domain socketのみを使う。
- 🟢 [Done] GUI起動時に所有権がworkerにあれば接続する。GUIはDBを開かず、GUIを終了してもworkerは継続する。workerへ接続できなければ別Coreへ切り替えない。
- 🟢 [Done] 3言語の未対応操作・接続断表示。接続断後は毎秒再接続を試みる。パネル配置はRustのRenderer表示状態としてGUI全ウインドウで共有する。
- 🟢 [Done] `save_settings`を共有コアで検証・保存し、GUI・IPCの両経路から呼び出す。遅延した再送は保存済み応答だけを返し、新しい設定を巻き戻さない。
- 🟢 [Done] 手動定義更新の非同期受付、共有進捗・診断、終了時の停止と子プロセス終了待機。
- 🟢 [Done] 隔離詳細・整合性確認・停止・確認結果取得。GUIの既存詳細画面と進捗・停止を使用する。
- 🟢 [Done] 隔離・復元・転送復旧の非同期IPC。既存の検証・ジャーナル・上書き拒否を共有コアで実行し、GUIは結果を待って既存画面を更新する。
- 🟢 [Done] 履歴・隔離・診断JSONのworker内生成と保存、ダウンロード容量確認・削除。ネイティブ保存先選択はGUIが担当。
- 🟢 [Done] エンジン再確認の非同期IPCと、fork継承FDに起因する通常終了後の所有権取得エラーの修正。
- 🟢 [Done] 起動中GUIからのworker起動・所有権交代とGUI復帰。実行中参照・処理・準備済みInstallerがある場合は拒否し、切替時にパネル配置を保持。
- 🟢 [Done] workerの予約claim・missed判定・定義自動更新。GUIと同じ共有スケジューラーを使用。
- 🟢 [Done] LaunchAgent登録・解除、OS許可・読み込み状態の表示と、許可変更後の復旧。
- 🟢 [Done] 署名判定・ネイティブフォーカス復帰を修正した0.3.3を公証・公開。公開0.3.2からの実更新・再起動も確認。
- 🟢 [Done] worker入口を含む正式0.3.2の署名・公証。署名済み0.3.3候補でLaunchAgentの登録・解除・許可変更後の復旧を実機確認。
- 🟢 [Done] 公開0.3.3で稼働中workerを伴うApplications間の往復移動・GUI修復と旧コピーが残る場合の拒否を実機確認。
- 🟢 [Done] 未読み込み時の修復・解除を0.3.4へ収録し、公証済みGUIで実機検証・GitHub公開。公開0.3.3からの実更新・再起動も成功。
- 🟠 [Next] ログイン後の自動起動・予約実行を実機確認。
- ⭕️ [Pending] worker固有のアクセス権限の検証。
- ⭕️ [Pending] helper固有のFull Disk Access、別ユーザーからの実接続拒否、ログイン・スリープ・macOS 12 / Intelでの確認。

## 開発用起動

既存のGUIを使用中のデータ領域にworkerを追加起動すると`data_in_use`になる。開発時は専用ディレクトリを使う。workerは`CLAMAV_DESKTOP_DEV_DATA_DIR`を読まず、指定したパスだけを使う。

```sh
cargo build --manifest-path src-tauri/Cargo.toml -p clamav-desktop-core --bin clamav-worker --locked
src-tauri/target/debug/clamav-worker serve --data-dir /private/tmp/clamav-worker-dev
```

状態取得の例。UUIDとissuedAtは要求ごとに生成する。コマンドの種類は`get_snapshot`、`start_scan`、`stop_scan`、`save_settings`、`update_definitions`、`get_quarantine_detail`、`verify_quarantine_integrity`、`stop_integrity_check`、`get_integrity_result`、`quarantine_file`、`restore_file`、`recover_file_transfers`、`get_file_operation_result`、`get_download_cache`、`cleanup_download_cache`、`export_data`、`refresh_engine`、`get_engine_inspection_result`、`shutdown`、`shutdown_if_idle`。`start_scan`には`kind`（quick/full/custom）と`target`（customの絶対パス、他はnull）を指定する。

```json
{
  "version": 1,
  "id": "a031772c-3549-4d9c-8fe2-74f0ab91a402",
  "issuedAt": "2026-10-13T11:00:00Z",
  "body": { "command": "get_snapshot" }
}
```

この構造をJSONファイルに保存し、日時を現在時刻に更新して以下へ渡す。

```sh
src-tauri/target/debug/clamav-worker request --data-dir /private/tmp/clamav-worker-dev < request.json
```

終了は`shutdown`要求、SIGINT、SIGTERM。検査に停止を要求して既存の応答と子プロセスの終了を待つ。SIGKILL後は次の所有者が履歴を中断として復旧し、残った私有ソケットを置き換える。設定の「GUIへ戻す」は、workerがアイドルであることを原子的に確認して終了し、所有権ロック解放後にローカルCoreを作る。切断だけでは自動でローカルへ戻らない。

## 通信契約

| 項目 | 値 |
| --- | --- |
| IPC版 | 1 |
| フレーム | 4バイトのbig-endian長 + UTF-8 JSON、接続ごとに1要求/1応答 |
| 要求上限 | 64KiB |
| 応答上限 | 8MiB、超過時は小さいエラー応答 |
| 同時接続 | 最大8、超過接続は切断 |
| 読取・書込・接続期限 | 各10秒 |
| 保存ディレクトリ / socket | ユーザー所有、0700 / 0600 |
| 認証 | 接続の両側でOSのpeer UIDを実効UIDと比較 |
| 再送受付 | issuedAtから24時間、未来時刻は5分以内 |
| 変更要求の記録 | SQLiteに最大1024件、期限まで保持 |

変更要求は実行前にIDと内容を保存し、応答も保存する。同じID・内容の再送には保存済み応答を返し、検査を再実行しない。違う内容でのID再使用は拒否する。実行と応答保存の間にクラッシュした場合は`ipc_outcome_unknown`を返し、状態・履歴の確認を必要とする。期限内の記録を押し出して再実行することはない。上限到達時は新しい変更要求を拒否し、状態取得は維持する。期限の更新で古い要求を再送せず、新たな操作には新しいUUIDを使う。

OSの所有権ロックを保持したCoreだけがsocketを作る。同じCoreでの二重bindも拒否する。終了時は自身が作ったinodeのsocketだけを削除する。リンク・通常ファイル・公開権限の既存socketを勝手に削除しない。任意のシェル・SQL・実行ファイルを指定する命令はない。

English: This is a manually started development worker with authenticated local IPC and startup-time GUI attachment. Live GUI ownership switching is implemented. Scheduling is supported while the worker runs; login service registration, OS permission status and recovery are implemented. Public 0.3.2 incorrectly rejects valid app signatures during registration; 0.3.3 is now notarized and published, with the same code-signing hash as the candidate that passed live registration, removal and OS permission recovery checks. Updating from public 0.3.2 and restarting also passed. Engine installation and app replacement require local mode.

简体中文：目前为手动启动的开发版后台进程，支持经过身份检查的本地通信及 GUI 启动时连接。已支持 GUI 运行中的所有权切换。后台运行期间已支持定时执行，已支持登录服务注册、系统许可状态及恢复。公开版 0.3.2 在注册时会错误拒绝有效签名，0.3.3 已公证并发布，与通过实机注册、注销及系统许可恢复验证的候选版具有相同的代码签名哈希；从公开版 0.3.2 更新并重启也已验证；安装引擎与替换应用需要本地模式。

## 設定保存（2026-10-16）

`save_settings`は`settings`にスナップショットと同じcamelCase形式の設定オブジェクトを渡す。言語（en/ja/zh-CN）、時刻（0–23）、最大ファイル容量（1–4096 MiB）、保持期間（1–365日）、取得容量上限（257–16384 MiB）を共有コアで検証する。DB保存が成功してからDocumentへ反映し、GUIは通知・次のスナップショット取得で確認する。成功応答は`ack`。GUIはDBを開かない。

IPC v1への追加コマンドであり、既存4コマンドの形式は変更しない。設定保存には更新後のGUIとworkerを組み合わせる。旧workerは追加コマンドを処理できない。

当初は設定保存のみだったが、2026-10-25にworker内の自動定義更新と毎日の予約検査を実装した。実行条件は下記「予約実行」を参照。

English: Settings can now be saved through authenticated IPC, using the same Rust validation and persistence as local mode. Duplicate requests never overwrite newer settings. Scheduling was subsequently implemented on 2026-10-25.

简体中文：现在可通过经过身份检查的本地通信保存设置，与本地模式共用 Rust 校验及持久化逻辑。重复请求不会覆盖较新的设置。后台定时任务已于 2026-10-25 实现。

## 手動定義更新（2026-10-17）

`{"command":"update_definitions"}`は、更新制限と処理中状態を検証して操作を確保し、`ack`を返す。`ack`は開始受付であり更新成功を意味しない。ダウンロード出力は`get_snapshot`の`operation`、最終結果は`diagnostics`の`definitions_update`で確認する。GUIの既存の更新ボタン・進捗・再試行表示が同じIPCを使う。

更新ジョブはworkerが保持するため、GUI終了や要求接続の切断では中断しない。同じ要求IDの再送は記録済み応答を返すだけで、完了済み・失敗済み更新を再実行しない。開始と応答保存の間のクラッシュは既存の`ipc_outcome_unknown`規則に従う。新しい再試行には新しい要求IDを使う。

長時間処理はIPC命令ロックを保持しない。状態取得・設定保存・検査停止・worker終了を処理できる。更新中の追加更新や検査開始は共有操作状態により拒否する。終了時は定義更新専用の停止フラグを立て、子プロセスを停止・回収してジョブ終了を待つ。上限30秒で応答しなければジョブを中断して`worker_stop_timeout`とする。停止結果は診断へ保存し、3言語で再試行を案内する。

自動定義更新・予約実行は2026-10-25に追加。アプリ自体のソフトウェア更新のIPC対応は未実装。

English: Manual definition updates now return an acceptance acknowledgement immediately. Progress and final diagnostics remain worker-owned; closing the GUI does not stop the job. Shutdown cancels and awaits the update child. Automatic scheduling was added on 2026-10-25.

简体中文：手动更新病毒定义现在会立即返回已受理响应。进度和最终诊断由后台进程管理，关闭 GUI 不会停止更新。后台退出时会停止并等待更新子进程。自动定时更新已于 2026-10-25 实现。

## 隔離詳細・整合性確認（2026-10-17）

`get_quarantine_detail`は`id`で記録を選び、ファイル状態・転送状態・最終整合性結果を返す。読み取りは再送記録を消費しない。ファイル操作中は待ち続けず`busy`を返す。任意のパスやファイル内容は渡せない。

`verify_quarantine_integrity`は`id`を受け取り、記録・処理中状態・ファイル操作ロックを確認して操作を確保した後に`ack`を返す。確認はworker所有のblocking taskで実行する。`stop_integrity_check`で停止を要求でき、進捗はスナップショットの`operation`へ反映する。GUIを閉じても確認処理は続く。

`get_integrity_result`の`request_id`には確認開始要求のUUIDを渡す。`integrity_result`の`result`は処理中ならnull、完了ならRust ResultのJSON形式（`{"Ok":null}`または`{"Err":"理由"}`）。workerは直近64件をメモリー上で保持する。再起動・履歴からの追い出し後は`ipc_outcome_unknown`となるため、隔離詳細・保存済み確認結果を確認する。確認完了後も開始要求の再送は保存済み受付応答だけを返し、再確認しない。

GUIのTauriコマンドはこの結果を待ってから詳細を返すため、既存UIの契約を維持する。確認結果のDB保存に失敗した場合やジョブ異常終了も成功扱いせず返す。

終了時は停止要求を出し、確認と結果保存の完了までCore所有権を保持する。OS内で停止しているディスクI/Oを安全に中断できないため、この場合は終了待ちが長引く。別Coreの起動やDBの同時所有による回避はしない。

English: Worker mode now supports quarantine details and cancellable integrity checks. Start acknowledges ownership; final results are queried by request UUID. Save failures propagate to the GUI. Shutdown retains Core ownership until the blocking filesystem task exits.

简体中文：后台模式现支持隔离详情和可停止的完整性检查。开始响应表示已受理，最终结果按请求 UUID 查询。保存失败会返回 GUI。退出时会保留数据所有权，直到文件检查任务结束。

## 隔離・復元・転送復旧（2026-10-17）

`quarantine_file`と`restore_file`は既存の記録IDを`id`に指定する。`recover_file_transfers`は追加引数を持たず、worker所有DBの未確定転送を復旧する。任意のパス、任意の実行ファイル、SQLは受け付けない。

ファイル操作ロックと共有操作状態を確保した後に`ack`を返す。これは受付であり、成功通知ではない。`get_file_operation_result`に開始要求の`request_id`を渡し、`file_operation_result.result`がnullなら処理中、`{"Ok":null}`なら成功、`{"Err":"理由"}`なら失敗。直近64件の結果をメモリーに保持し、再起動や追い出し後は`ipc_outcome_unknown`を返す。元の開始要求は既存の永続台帳で抑止し、完了・失敗後も同じIDの再送では操作をやり直さない。新しい再試行は新しい要求IDを使う。

GUIは短い結果取得を繰り返し、完了結果を既存のTauriコマンドへ返す。GUI側でDBやファイル転送を実行せず、閉じたり切断したりしてもworker内の操作は継続する。復元先の既存ファイルやハッシュ不一致は既存の共通コアで拒否する。

通常GUIモードも同じ操作ラッパーを使い、処理中・検査中の追加操作を拒否する。共有操作にはquarantine/restore/transfer_recoveryと対象IDを保持し、3言語の段階表示を使う。転送容量の数値進捗は今回追加していない。

ファイル操作は途中で取り消さない。workerの終了要求は受け付けるが、操作とジャーナル処理が完了するまで終了・Core所有権の解放を待つ。ディスクI/OやClamAV再検査に時間がかかる場合は終了待ちも長くなる。強制終了した場合は既存の転送ジャーナル復旧に従う。実機抜き差し、exFAT実ボリュームの検証は引き続き保留。

English: Quarantine, restore and transfer recovery now run in worker-owned jobs. Acknowledgements mean acceptance; request-correlated results propagate failures to the existing GUI. Retried requests never repeat file mutations. Shutdown waits for accepted operations and journal work to finish.

简体中文：隔离、恢复和传输修复现由后台任务执行。响应表示已受理，GUI 按请求 ID 获取最终结果及错误。重复请求不会重复移动文件。退出时会等待文件操作和日志处理完成。

## 出力・容量管理（2026-10-17）

`get_download_cache`は容量・削除可能容量・件数を返す読み取り要求。ファイル操作中は`busy`を返す。`cleanup_download_cache`は既存のファイルジョブとして受付後に`ack`を返し、`get_file_operation_result`で完了結果を確認する。保持日数・容量設定は共有コアから取得する。準備済みパッケージ、不明ファイル、リンクは既存ポリシーに従って保護する。GUIの削除コマンドは完了後の容量情報を返す。

`export_data`には`kind`（history/quarantine/diagnostics）、絶対パスの`destination`、任意の`app_update`メタデータを指定する。GUIでは保存ダイアログで選択したパスだけを渡す。JSON生成・保存はDB所有workerで実施し、内容やファイル本体をIPC/WebViewへ返さない。開始受付・結果取得・再送・終了待機は既存ファイルジョブと同じ。

履歴は既存の配列形式、隔離・診断はschemaVersion=1を維持する。診断のappUpdateはGUIのRust側が渡した任意の状態スナップショットであり、worker内のアップデーターではない。メタデータはオブジェクト・48KiB以下とし、全要求は既存64KiB上限で制限する。

通常GUIモードも同じ共通コアの出力処理を使う。出力は0600の一時ファイルへ書き込み・同期してから置換し、失敗時は一時ファイルを削除する。アプリ領域、隔離元、検査履歴や現在の検出元、リンクへの上書きを拒否する。履歴出力もこの保護・権限を適用する。既存の正常な出力ファイルは保存ダイアログの確認に従って置換できるが、同じ開始要求の再送では書き直さない。

English: History, quarantine and diagnostics exports are generated and saved by the worker. IPC transfers only a destination, bounded metadata and status. Cache reports and cleanup now work remotely; prepared/unknown files remain protected. Native save dialogs stay in the GUI.

简体中文：历史、隔离及诊断 JSON 由后台生成并保存，IPC 仅传递目标路径、有限的元数据及状态。现支持远程缓存容量查询和清理，保留已准备的安装包及未知文件。保存对话框仍由 GUI 提供。

## エンジン再確認と所有権解放（2026-10-17）

`refresh_engine`は共有操作状態を確保して`ack`を返す。`get_engine_inspection_result`に開始要求の`request_id`を渡し、`engine_inspection_result.result`がnullなら確認中、`{"Ok":{...Engine...}}`なら完了、`{"Err":"理由"}`なら失敗。結果は直近64件を保持する。再送は既存台帳の受付/拒否を返すだけで再確認しない。結果が消えた場合は`ipc_outcome_unknown`とし、スナップショットと診断を確認する。

GUIの既存の再確認・再試行は同じTauriコマンドを使い、結果まで待つ。確認段階は3言語で表示する。確認中も状態取得・検査停止・終了要求を処理し、追加操作はbusyで拒否する。終了時はローカルprobeの終了を最大30秒待つ。ネットワーク更新は行わず、エンジンのversion照会と定義ファイルの存在確認を行う。

所有権ロックはCLOEXECを維持しつつ、CoreがDB/Documentを破棄した最後に取得PIDからLOCK_UNする。forkで継承されたFDはexec前まで残り得るため、FDのcloseだけに解放を依存しない。継承側PIDからのDropでは親のロックを解除しない。実際に稼働中の所有者、別プロセス、同一inodeへの排他は維持し、ロックファイルは削除しない。根拠：[Apple flock(2)](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/flock.2.html)。

English: Engine rechecks now use asynchronous IPC with request-correlated results and shutdown waiting. Normal Core destruction explicitly unlocks from the acquiring PID after DB teardown, preventing fork-inherited pre-exec descriptors from delaying ownership release. Active owners still exclude contenders.

简体中文：引擎重新检查现通过异步 IPC 执行，并按请求 ID 获取结果。Core 正常销毁时由取得锁的进程在关闭数据库后显式解锁，避免 fork 后 exec 前的继承文件描述符延迟释放。活动进程的排他保护仍然保留。


## 起動中の所有権交代（2026-10-17）

設定の「実行プロセス」からGUI→worker→GUIを切り替える。`Runtime`がBackendへの参照を管理し、コマンド・maintenance・非同期ジョブが参照を持つ間はbusyで拒否する。切替中は新しい参照の取得とGUIの通常終了を拒否する。maintenanceは待機中にCoreを保持せず、worker接続中は自動処理を行わない。

workerは別helperではなく、現在のアプリ実行ファイルを`--worker serve --data-dir ABSOLUTE_PATH`で起動する。Tauri初期化前に`core::worker`へ分岐し、専用2スレッドruntimeで動作する。実行ファイル・データパスはRustが決め、WebViewから受け取らない。独立したプロセスグループと閉じた標準入出力でGUI終了後も稼働する。単体`clamav-worker`も同じ入口を使用する。ログイン登録は追加していない。

GUI→workerはCore/DBを破棄して所有権を解放した後に起動し、認証済みSnapshotを受信して接続完了とする。起動失敗・早期終了時はOSロックを取得できる場合だけGUIへ戻す。15秒以内に応答しない生存workerを強制終了せず、workerプロキシで再接続を継続する。

worker→GUIは新しい`shutdown_if_idle`を使う。変更要求と同じゲート内で処理中判定と停止フラグを設定するので、同時クライアントのジョブを誤って停止しない。busyも再送台帳へ保存する。ack後もCore生成に成功するまで最大15秒待ち、ロックを奪わない。切断済みworkerからの明示的なGUI復帰もOSロック取得が必要。旧workerは追加コマンド未対応のため、手動終了後に更新版を起動する。

設定・履歴・隔離記録はSQLiteから読み直し、ログパネルの左右・独立表示はGUIのRust状態から引き継ぐ。セッション中の操作ログは所有者変更でリセットされる。準備済みInstallerの保護を失わないよう、その状態では切替を拒否し3言語で案内する。未保存の設定はworkerへ送らない。GUIへ戻った際はエンジンを即時再確認する。

English: Settings now supports live app/worker handoff. Active leases and operations block transfer; startup failure recovers only with exclusive ownership. The application executable hosts the same headless worker before Tauri starts. Atomic idle shutdown protects concurrent jobs. Login startup and signed-release validation remain pending; scheduling was added on 2026-10-25.

简体中文：设置现支持应用与后台进程之间实时移交。活动引用或操作会阻止切换；启动失败时仅在取得独占锁后恢复。应用可执行文件在 Tauri 初始化前运行共享后台入口，空闲检查与停止为原子操作。登录启动和正式签名发行版验证仍待完成；定时任务已于 2026-10-25 实现。

## 予約実行（2026-10-25）

GUIローカルモードとworkerは、共有コアの同じスケジューラーを30秒ごとに評価する。worker接続中のGUIは実行せず、所有者だけが予約と定義更新を開始する。起動済みworkerはGUIを閉じた後も実行するが、OSのログイン登録・スリープ解除は行わない。

予約は毎日のquick検査。設定したローカル時刻から1時間の開始枠を設け、枠内では他操作・エンジン確認・定義不足・更新制限の解消を待つ。期限を過ぎたらmissedとして保存し、後から埋め合わせない。枠内で開始した検査は完了まで継続する。DSTの重複時刻は最初の時刻を採用し、存在しない時刻はmissedとする。

SQLiteのscheduled_runsはローカル日付と種別を一意キーとする。開始前にscan ID付きclaimをコミットし、時刻・設定の変更、所有権交代、再起動でも同じ日の実行済み予約を再実行しない。起動前の競合や時計変更ならpendingに戻して再判定し、実際に開始した検査は再試行しない。再起動時は保存済みの検査結果と照合し、結果不明のclaimはinterruptedとして残す。検査履歴と予約結果は同一トランザクションで保存する。旧last_scheduled_scanも重複抑止用に移行する。

予定UTC時刻、実行期限、ローカル日付・時刻、タイムゾーンの説明ラベル（現在のUTCオフセット）、設定全体のSHA-256、開始・終了時刻、状態・理由、scan IDを保存する。IANAタイムゾーン名は保存しない。稼働中に観測した日だけを記録し、電源オフ中の全日付を生成しない。設定画面は最新の日付の保存済み予約を3言語で表示し、診断JSONのscheduledRunsには全記録を含める。

定義自動更新は最終開始試行から6時間以上経過し、エンジン確認済み・アイドル・待機期限外の場合に開始する。実行可能な予約検査を優先する。更新の操作確保と開始試行の保存に成功してから子プロセスを起動し、保存失敗なら操作を解放する。workerの既存の所有ジョブとして動作し、終了要求と同じゲートで直列化し、終了時に停止・回収を待つ。実CDNへの自動更新は今回の検証対象外。

English: Daily quick scans now run in either the GUI owner or an already-running worker. Durable daily claims prevent replay across handoff, restart and clock changes. The start window is one hour; missed scans are not caught up. Automatic definitions updates use the shared operation gate and six-hour attempt interval. Login startup is not registered.

简体中文：每日快速扫描现可由 GUI 所有者或已运行的后台进程执行。每日持久化记录可防止切换、重启或时钟变化后的重复执行。仅在预约时间后的一小时内开始，错过后不补扫。病毒定义自动更新共用操作锁，尝试间隔为六小时。尚未注册登录启动服务。


## ログイン登録の事前確認（2026-11-04）

新しいビルドを安定したApplications配下へ配置した後、以下でGUIを開かずに登録候補を確認できる。

```sh
'/Applications/ClamAV Desktop.app/Contents/MacOS/clamav-desktop' --login-service-check
```

この入口は新規実装のため、公開済み0.3.1には含まれない。本人のApplicationsへ導入した場合はその絶対パスを使う。開発用バイナリや署名条件を満たさない.appは拒否される。出力の`plist`は登録候補であり、OSへの登録・起動・停止・設定変更は行わない。`registration`は`absent`／`matches`／`conflict`、`permission`は`unknown`、`loaded`は`null`。ホーム／データパス等を含むので、診断共有時は個人情報を確認する。[検証条件と残作業](background-service-plan.md#launchagent事前検証2026-11-04--done)。

English: The new `--login-service-check` entry emits a read-only registration plan from an installed, Developer ID signed app; it is not available in the published 0.3.1. It does not enable login startup. Permission and running state remain unknown.

简体中文：新增的 `--login-service-check` 入口从已安装且具有 Developer ID 签名的应用输出只读注册计划；已发布的 0.3.1 不包含此入口。它不会启用登录启动，许可和运行状态仍为未知。


## 明示的なログイン登録・解除（2026-11-04）

新しい署名済みビルドをApplicationsへ配置し、一度通常起動してデータ領域を準備した後に使う。公開済み0.3.1にはこの入口を含まない。登録前はGUIと手動workerを終了する。GUIがデータを所有したまま有効化すると`data_in_use`で停止する。

```sh
'/Applications/ClamAV Desktop.app/Contents/MacOS/clamav-desktop' --login-service enable
'/Applications/ClamAV Desktop.app/Contents/MacOS/clamav-desktop' --login-service disable
```

`enable`は登録・開始後にworkerのIPC応答を待つ。`disable`はアイドル終了と所有権解放を確認して登録を外す。検査・隔離・復元などの実行中は`busy`で拒否し、処理の完了後に再実行する。GUIがworkerへ接続中の場合も、操作の混乱を避けるためGUIを閉じてから解除する。

登録失敗時は`login_service_enable_failed`／`login_service_bootstrap_failed`／`login_service_worker_start_timeout`等を返し、既知の登録ファイルを保持する。既に読み込み済みの登録は自動で置換しない。GUIを閉じた状態で`disable`が成功してから`enable`し直す。不明なファイルや移動前のパスとの競合は`login_service_registration_conflict`で停止し、自動で修復・削除しない。OSの許可状態・実際のログイン後動作は別途確認が必要で、事前確認の`permission=unknown`と`loaded=null`は変えていない。

この段階のOS登録・解除は模擬アダプターで検証しており、実環境のログイン設定を変更していない。実worker IPCでのbusy拒否、終了待機、解除前の排他取得を確認した。正式配布ビルドからの実登録は⭕️ [Pending]。

English: The new `--login-service enable|disable` commands manage this user's fixed LaunchAgent only. Close the GUI/manual worker before enabling. Disabling rejects active work, waits for idle shutdown and exclusive ownership, and preserves all app data. Actual signed-app OS registration remains pending.

简体中文：新增的 `--login-service enable|disable` 命令仅管理当前用户的固定 LaunchAgent。启用前请退出 GUI 和手动后台进程。移除时拒绝正在进行的操作，等待空闲退出和独占锁，并保留全部应用数据。正式签名应用的实际系统注册仍待验证。


## GUIでログイン起動を管理する（2026-11-04）

新しいビルドの「設定 → 実行プロセス → ログイン時に起動」から有効化／登録解除／再確認を行う。Applications内のDeveloper ID署名済みアプリと通常データ領域を要求し、開発targetや不明な登録では有効化しない。予約時刻などの設定変更は先に保存する。

GUI内Coreを安全に解放してから登録するため、GUIを閉じてCLIを実行する必要はない。手動workerを使用中なら、そのアイドル終了を待ってLaunchAgentへ引き継ぐ。解除後はGUI実行へ戻る。操作・参照保持中はbusyで拒否し、失敗時もロックを奪わない。登録したままの手動worker切替やアプリ更新は拒否するため、先に登録を解除する。

「登録済み」は登録ファイルの一致を表し、OS許可や次回ログイン起動は未確認と表示する。不一致の登録は変更しない。実際の署名済み配布.appでの登録・解除・ログイン検証は⭕️ [Pending]。この機能は公開済み0.3.1には含まれない。

English: Use Settings → Execution → Login startup in the new build. Rust handles ownership transfer, idle-only worker shutdown and recovery; close-and-run-CLI is no longer required. Registration and OS permission remain separate states. Signed-app lifecycle checks are pending.

简体中文：新构建中可使用“设置 → 执行进程 → 登录时启动”。Rust 负责所有权移交、空闲退出及失败恢复，无需先退出 GUI 再运行命令。注册状态与系统许可分开显示。正式签名应用的生命周期验证仍待完成。


## OS状態の確認と移動修復（2026-11-04）

設定の「OSの許可」と「読み込み」は別の状態。macOS 13以降は公式APIで許可／許可待ち等を取得し、macOS 12や未照会は未確認とする。読み込み済みはlaunchdに定義がある状態で、workerの稼働やフルディスクアクセスとは別。照会失敗は未読み込みと断定しない。「許可が必要」の場合は「ログイン項目」からOS設定を開き、本人が承認した後に再確認する。起動しない場合は登録解除→有効化で再試行する。

アプリをシステムのApplicationsと本人のApplicationsの間で移動し、旧実行ファイルがなく、登録が既知の形式に完全一致するときは「移動前の登録」と「登録を修復」を表示する。旧アプリが残る場合、任意フォルダへの移動、名前変更、不明な登録、リンクは自動修復しない。処理中は拒否し、旧登録をバックアップしてから現在の署名済みアプリへ切り替える。

```sh
'/Applications/ClamAV Desktop.app/Contents/MacOS/clamav-desktop' --login-service-check
'/Applications/ClamAV Desktop.app/Contents/MacOS/clamav-desktop' --login-service repair
```

本人のApplicationsを使う場合はその絶対パスを指定する。`--login-service-check`は読取専用で、`registration=repairable`、実際に照会した`permission`・`loaded`、`permissionSupported`・`osQueryFailed`も出力する。修復失敗時は旧内容または私有`.backup`を保持する。新しい登録が保存済みなら、解除→有効化で再試行できる。実際の署名済みアプリでの修復・許可変更は⭕️ [Pending]。

English: Settings and the read-only check now separate OS approval from launchd loading. Repair is limited to canonical private registrations for a missing old app in the other supported Applications directory. Approve via Login Items when required; refresh, then explicitly remove/re-enable if startup still fails. Real signed-app lifecycle tests remain pending.

简体中文：设置和只读检查现区分系统批准与 launchd 加载状态。仅可修复指向另一个受支持 Applications 目录中已不存在旧应用的标准私有注册。需要批准时打开登录项，由用户批准并刷新；仍无法启动时明确移除后重新启用。正式签名应用的生命周期验证仍待完成。


## 許可後の復旧（0.3.2）

ログイン項目で許可してアプリへ戻ると、許可と読み込み状態を自動で再確認する。状態の照会は重複させず、確認だけで登録やプロセスを変更しない。設定の「再接続・再起動」で明示的に復旧する。CLIは`--login-service recover`。

応答するworkerは検査中でもそのまま接続し、処理を止めない。接続できなければ許可待ちを拒否し、Core所有権を取得できるまで待つ。読み込み状態を確定できた場合だけ、既知の定義を読み直して起動し、認証済みSnapshotを確認する。強制kickstart、launchctl enableによる許可設定の上書き、無条件のCore起動は行わない。失敗時は登録・データを保持し、GUIが安全に所有権を取得できなければworkerプロキシを維持する。

公開用更新署名鍵の独立バックアップが済むまで、0.3.2の配布アプリ公開は保留する。既存0.3.1は引き続き使用でき、ソースとアプリ公開を区別する。

English: 0.3.2 adds a focus-refresh and an explicit reconnect/restart action. Healthy workers retain active jobs; restarting requires idle ownership and known loading state. Approval is never overridden. Signed-app publication awaits the independent successor-key backup.

简体中文：0.3.2 新增窗口恢复焦点时刷新状态，以及显式重新连接／启动。响应正常的后台进程保留当前任务；重新启动必须确认空闲、独占锁和加载状态，不覆盖系统批准设置。正式应用发布仍等待新签名密钥的独立备份。


## 移動修復の実機確認（2026-11-05）

日本語：公開0.3.3で、起動済みworkerを伴う`/Applications`と`~/Applications`間の往復移動・GUI登録修復に成功。旧アプリが残る場合は修復を拒否し、元の登録とPIDを保持する。一方、未読み込み状態では`launchctl bootout`の終了コード3を誤って失敗扱いする。Rust側は修正・実OS検証済みだが、配布0.3.3には未収録。元の配置へ戻し、登録が一致する状態で「再接続・再起動」を使う復旧経路を利用できる。データ保存先は削除しない。

English: Public 0.3.3 passed GUI registration repair after actual moves in both directions between `/Applications` and `~/Applications` with a running worker. An existing old app prevents repair without changing the registration or PID. An unloaded service exposes a `bootout` exit-code-3 bug. The Rust fix passed live OS tests but is not included in public 0.3.3. Restore the original app location and use reconnect/restart once the registration matches; retain the application data.

简体中文：公开版 0.3.3 已通过运行中后台进程随应用在 `/Applications` 与 `~/Applications` 之间双向移动后的 GUI 注册修复验证。旧应用仍存在时会拒绝修复，并保持原注册和 PID。未加载服务会触发 `bootout` 退出码 3 处理错误。Rust 修复已通过实机系统测试，但尚未包含在公开版 0.3.3 中。可恢复应用原位置，在注册匹配后使用重新连接／重启；请保留应用数据。


### 0.3.4での解消

日本語：上記の未読み込み時の不具合は、署名・公証済み0.3.4のGUIで登録修復・解除ともに実機検証済み。GitHub公開後、元の公開0.3.3からのアプリ内更新・自身による再起動も成功した。データは全件保持し、ログイン登録は未登録へ戻した。

English: The unloaded-service issue above is fixed in signed/notarized 0.3.4. Live GUI repair and removal both passed. After GitHub publication, an unmodified public 0.3.3 updated and restarted itself successfully. All saved data was preserved; login registration was removed after testing.

简体中文：上述未加载服务的问题已在签名、公证版 0.3.4 中修复，GUI 注册修复及注销均通过实机验证。GitHub 发布后，未修改的公开版 0.3.3 已成功通过应用内更新并自行重启。全部数据保留，验证后已注销登录服务。
