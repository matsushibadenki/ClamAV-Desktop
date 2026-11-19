# ログイン起動・予約実行の実機検証

## 確認できた範囲（2026-11-05、公開0.3.4）

- 🟢 [Done] GUI終了後も、LaunchAgentのworkerが認証付きIPCへ応答。
- 🟢 [Done] 手動の検査開始命令を送らず、保存した設定を共有スケジューラーが評価し、実際のDownloads検査を実行。3件・エラー0件・検出0件で完了。
- 🟢 [Done] 14時の開始枠内（14:59:44 JST）に始まり、15:00:08に完了。開始済み検査は枠の終了で打ち切られない。
- 🟢 [Done] worker再起動と15時への予約時刻変更後も同じ日付・scan IDを維持。評価周期を超える62秒後も二重実行なし。
- 🟢 [Done] 検証後は元の予約無効・12時・ログイン未登録へ復元。既存履歴・隔離を保持し、新しい正当な検査履歴と予約台帳を1件ずつ保存。
- ⭕️ [Pending] 実ログアウト・再ログインによる自動起動。今回はユーザー指定でセッションを切り替えていない。bootstrap、workerの再起動、画面ロックはログイン検証の代用にしない。

## 次回の実ログイン検証

1. 元の設定・登録状態、アプリの版・署名、worker PID・起動日時、履歴・隔離・予約台帳を非公開領域に記録する。公開するのは件数と検証結果のみとする。
2. 本日のquick予約が未実行であることを確認する。同じ日の実行済み・中断済みclaimは削除しない。2026-11-05分は今回の事前検証で完了済みなので、新しい予約のログイン起動検証は別の日に行う。
3. 公開0.3.4以降で、ログイン起動を登録し、予約検査を有効にして開始時刻を保存する。自動定義更新など他の設定は元の値を保持する。時刻は整数時間で、開始枠は現地の指定時刻から1時間。ログイン前に実行されないよう、余裕のある将来時刻を選ぶ。
4. GUIを終了し、許可済み・読み込み済み・worker応答を確認する。ユーザーが未保存の作業を保存してから、自分でログアウト・再ログインする。強制終了や自動ログアウトは行わない。
5. 再ログイン後はClamAV Desktopを手動起動しない。Codexだけを開いて検証を再開する。`--login-service-check`、固定ラベルのlaunchctl読み取り、`--worker request`による状態取得で、新workerの起動日時・PID・親プロセス、登録・許可・読み込み・IPC応答を記録する。`enable`、`recover`、`bootstrap`などの起動操作は証拠取得前に実行しない。
6. 指定時刻以後、GUIなしで予約台帳にscan IDと開始日時が保存され、同じIDの履歴が完了することを確認する。失敗・未完了・権限制限は成功に置き換えず、その結果を記録する。
7. 検査完了後、同日の二重実行がないことを確認する。元の設定・登録状態へ戻し、既存履歴と隔離を保持する。検証で作成された正当な検査履歴と予約台帳は削除しない。

## English

Preflight on public 0.3.4 verified a real scheduled Downloads scan with the GUI closed: 3 files, no errors or detections. No manual scan command was sent. A worker restart plus schedule-hour change did not repeat the daily scan. Original settings and the unregistered state were restored; the legitimate new history and daily claim were retained. Actual logout/login remains pending by the user's choice. On the next test, choose a day without an existing daily claim, save a future schedule, close the GUI and let the user log out/in after saving work. Collect read-only launchd/process/IPC evidence before launching the GUI or issuing any service-start command. Preserve real scan/ledger records and restore original settings and registration afterward.

## 简体中文

公开版 0.3.4 的事前验证已确认 GUI 关闭时真实执行 Downloads 预约扫描：3 个文件，无错误、无检出，未发送手动扫描命令。重启后台进程并更改预约小时后，同日扫描未重复执行。原设置及未注册状态已恢复，新增的真实扫描及预约记录保留。按用户选择，本次未注销／重新登录，实际登录启动仍待验证。下次请选择当天尚无预约执行记录的日期，保存将来时刻的预约并关闭 GUI；由用户保存工作后自行注销、登录。在手动启动 GUI 或发送服务启动命令之前，先以只读方式记录 launchd、进程和 IPC 证据。保留真实扫描及预约记录，完成后恢复原设置及注册状态。

## worker起動記録（2026-11-06、ソース実装）

新しいworkerは、Coreの所有権を取得してIPCソケットを準備した時点で起動記録を保存する。既存の診断JSON出力（GUIまたはworkerのExportData/diagnostics）に`workerRuntime`を追加した。公開0.3.4には未収録。

| 項目 | 意味 |
| --- | --- |
| `current` | 診断を生成するworkerの記録。GUIがCoreを所有する場合はnull。 |
| `recent` | SQLiteに保存した新しい順の最大16件。未記録は空配列、読み取り不能はnull。 |
| `instanceId` | workerの準備ごとに生成するUUID。PID再利用と区別するために使う。 |
| `pid` / `parentPidAtReady` | 記録時のPIDと親PID。親PIDは後からの再親化を追跡する値ではない。 |
| `readyAt` | IPCソケット準備後のUTC日時。OSのプロセス作成日時やログイン日時ではない。 |
| `appVersion` | workerコアのビルド版。署名・公証の照合は別途行う。 |
| `journalPersisted` | この記録の保存が成功したか。falseなら過去記録との比較で保存済みと扱わない。 |

診断の`collectionErrors`には、保存不能・読み取り不能・破損を固定コードで記録する。破損した記録を上書きせず、記録の保存失敗だけでworkerの検査機能を止めない。記録に環境変数・起動引数・ファイル内容・認証情報は含めない。SQLiteの予約台帳は変更しない。

次回検証では、ログアウト前と再ログイン後の診断を非公開で保存し、`instanceId`の交代、`readyAt`、実launchd PIDと予約台帳のscan IDを照合する。この記録だけでログイン自動起動成功とは判断しない。手動起動・bootstrap・recoverでも新しい記録は作られる。実ログインの実施と、証拠採取前に起動操作をしていないことの確認が引き続き必要。

English: The source implementation adds `workerRuntime` to diagnostic exports, with the current worker instance and up to 16 persisted readiness records. A UUID distinguishes instances even when PIDs are reused; `readyAt` is socket-readiness time, not OS login/process creation time. Local GUI exports have `current=null`. Persistence/read/corruption errors are reported without blocking scanning or replacing corrupt evidence. No arguments, environment or file contents are recorded. Compare private pre/post-login reports with launchd and scheduled-run evidence; these records alone do not prove login startup. This addition is not in public 0.3.4.

简体中文：源码实现为诊断导出添加 `workerRuntime`，包含当前后台实例及最多 16 条持久化准备就绪记录。UUID 可区分 PID 被重复使用的不同实例；`readyAt` 是套接字准备就绪的时间，并非系统登录或进程创建时间。GUI 本地导出时 `current=null`。保存、读取或记录损坏错误会被报告，不阻止扫描，也不覆盖损坏的原记录。不保存启动参数、环境变量或文件内容。登录前后应私下保存诊断并结合 launchd 及预约记录比对；仅凭这些记录不能证明登录自动启动成功。公开版 0.3.4 尚未包含此功能。

## GUIを開かずに診断を採取する

リポジトリで `node scripts/capture-login-evidence.mjs` を実行する。個人用Applicationsにインストールした場合は、署名済みアプリの絶対パスを引数に指定する。ログアウト前と再ログイン後のJSONを私有領域に保存して比較する。PIDが採取中に交代した場合は再取得する。実ログインの時刻と照合し、自動起動と予約実行を判定する。

English — Run `node scripts/capture-login-evidence.mjs` from the repository to collect registration, launchd, IPC and the latest scheduled run without opening the GUI. Pass the absolute installed app path if needed. Compare private captures before logout and after login with the actual login time.

简体中文 — 在仓库中运行 `node scripts/capture-login-evidence.mjs`，无需打开 GUI 即可采集注册、launchd、IPC 及最近的预约执行状态。必要时传入已安装应用的绝对路径。请私下保存注销前与登录后的记录，并与实际登录时间比对。
