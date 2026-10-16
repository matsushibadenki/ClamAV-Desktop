# Worker / local IPC

現在は開発用の手動起動段階。ログイン時の登録、アプリ終了中の予約、常駐設定の切り替えUIは未実装。

## 実装済み

- 🟢 [Done] Tauriに依存しない`clamav-worker`。専用データ領域を明示し、Coreの所有権を取得してからソケットを公開する。
- 🟢 [Done] 状態取得、検査開始、検査停止、設定保存、worker終了。同じユーザーのUnix domain socketのみを使う。
- 🟢 [Done] GUI起動時に所有権がworkerにあれば接続する。GUIはDBを開かず、GUIを終了してもworkerは継続する。workerへ接続できなければ別Coreへ切り替えない。
- 🟢 [Done] 3言語の未対応操作・接続断表示。接続断後は毎秒再接続を試みる。パネル配置はRustのRenderer表示状態としてGUI全ウインドウで共有する。
- 🟢 [Done] `save_settings`を共有コアで検証・保存し、GUI・IPCの両経路から呼び出す。遅延した再送は保存済み応答だけを返し、新しい設定を巻き戻さない。
- 🟠 [Next] 定義更新、隔離・復元・整合性確認・出力等のIPC対応と、起動中GUIの有効化/無効化による所有権交代。現時点でGUIのこれらの操作はworker接続中に未対応エラーを返す。
- 🔴 [Later] helperの.app同梱・署名、LaunchAgent登録、予約実行。
- ⭕️ [Pending] helper固有のFull Disk Access、別ユーザーからの実接続拒否、ログイン・スリープ・macOS 12 / Intelでの確認。

## 開発用起動

既存のGUIを使用中のデータ領域にworkerを追加起動すると`data_in_use`になる。開発時は専用ディレクトリを使う。workerは`CLAMAV_DESKTOP_DEV_DATA_DIR`を読まず、指定したパスだけを使う。

```sh
cargo build --manifest-path src-tauri/Cargo.toml -p clamav-desktop-core --bin clamav-worker --locked
src-tauri/target/debug/clamav-worker serve --data-dir /private/tmp/clamav-worker-dev
```

状態取得の例。UUIDとissuedAtは要求ごとに生成する。コマンドの種類は`get_snapshot`、`start_scan`、`stop_scan`、`save_settings`、`shutdown`。`start_scan`には`kind`（quick/full/custom）と`target`（customの絶対パス、他はnull）を指定する。

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

終了は`shutdown`要求、SIGINT、SIGTERM。検査に停止を要求して既存の応答と子プロセスの終了を待つ。SIGKILL後は次の所有者が履歴を中断として復旧し、残った私有ソケットを置き換える。workerを終了した後、GUIを再起動するとローカルCoreに戻る。接続中GUIからの自動切り替えはまだ行わない。

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

English: This is a manually started development worker with authenticated local IPC and startup-time GUI attachment. Scheduling, service registration, full command migration and live GUI ownership switching remain unfinished.

简体中文：目前为手动启动的开发版后台进程，支持经过身份检查的本地通信及 GUI 启动时连接。定时执行、服务注册、全部命令迁移及 GUI 运行中的所有权切换尚未完成。

## 設定保存（2026-10-16）

`save_settings`は`settings`にスナップショットと同じcamelCase形式の設定オブジェクトを渡す。言語（en/ja/zh-CN）、時刻（0–23）、最大ファイル容量（1–4096 MiB）、保持期間（1–365日）、取得容量上限（257–16384 MiB）を共有コアで検証する。DB保存が成功してからDocumentへ反映し、GUIは通知・次のスナップショット取得で確認する。成功応答は`ack`。GUIはDBを開かない。

IPC v1への追加コマンドであり、既存4コマンドの形式は変更しない。設定保存には更新後のGUIとworkerを組み合わせる。旧workerは追加コマンドを処理できない。

自動定義更新と毎日の予約検査は設定として保持するが、手動workerにはまだスケジューラーがない。通常のGUIローカルモードへ戻るまで自動実行しない。3言語の情報アイコンのヘルプにもこの制約を記載。

English: Settings can now be saved through authenticated IPC, using the same Rust validation and persistence as local mode. Duplicate requests never overwrite newer settings. Scheduling in the manually launched worker is still unavailable.

简体中文：现在可通过经过身份检查的本地通信保存设置，与本地模式共用 Rust 校验及持久化逻辑。重复请求不会覆盖较新的设置。手动后台进程尚不支持定时任务。
