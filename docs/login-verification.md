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
