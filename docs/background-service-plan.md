# Background service implementation plan

共有コア・worker・予約実行は実装済み。OSへのログインサービス登録は未実装で、以下は残作業を含む設計・検証計画。

## 優先順位と構成

最初は既存のclamscanでアプリ終了中の予約検査を実現する。clamdは起動時間の短縮を目的とした任意の後続機能にする。常駐エンジンのメモリ負担を予約検査の必須条件にしない。

Rustの共有ライブラリがDocument、SQLite、検査、定義更新、隔離を所有する。Tauriは表示・ダイアログ・Rendererを担当する。バックグラウンド有効時は同じユーザーのRust workerが唯一のDocument所有者となり、TauriはローカルIPC経由で命令とSnapshotを取得する。各WebViewは表示状態だけを持つ。無効時は従来のGUI内Coreを使うが、所有権を取得するまでDBの初期化・復旧・変更を開始しない。

初期対象はログイン中のユーザー別LaunchAgent。管理者権限を必要とするLaunchDaemon、ログアウト中の検査、スリープ解除は対象に含めない。バックグラウンド有効化は既定でオフにする。macOS 12のサポートを維持するため、初期登録アダプターはユーザーのLaunchAgentsとlaunchctlを使用し、OS呼出しを共有コアから分離する。登録・解除はシェルを介さない引数配列で実装する。配布前にOS別の登録・許可状態を実機確認する。

## 1. 共有コアと所有権 — 🟢 [Done]

2026-10-13実装済み。`src-tauri/core`はTauriに依存しない独立crate。Coreが通知先の`StateNotifier`を持ち、GUIの`bridge.rs`がイベントへ変換する。`owner.lock`のOS排他をDB初期化・復旧前に取得する。別プロセスの競合、強制終了後の取得、一度だけの復旧、不正なロック、再入可能な通知をテストした。実ClamAVの無害な独自シグネチャでGUIなしの検査・隔離・整合性確認・復元も確認した。workerは次項のとおり実装済み。OSサービス登録は未実装。

- `store.rs`のCoreからAppHandle依存の通知を分離し、Snapshot通知用のインターフェースを導入する。`scanner.rs`、`engine.rs`、`quarantine.rs`も同じCoreと通知先を受け取る。Installerの画面起動とRendererはGUIに残す。
- 私有データ領域に通常ファイルの所有権ロックを作り、OSの非ブロッキング排他ロックをプロセスの存続期間保持する。0700の親、所有者、リンク拒否を確認する。PIDの存在や時刻だけでロックを奪わず、ロックファイルを実行中に削除しない。
- Core生成前に所有権を確定する。現在の「起動時にrunningをinterruptedへ変更」「転送ジャーナルの復旧」は所有者だけが行う。SQLite WALや現在のMutexをプロセス間排他の代わりにしない。
- 最初の成果物はGUIの動作を維持した共有コアと、別プロセスから同じ保存領域を開くとbusyになるテスト。workerを出荷する前に、二重起動・所有者クラッシュ後の取得・復旧の一回実行を検証する。

## 2. WorkerとIPC — 🟢 [Done] / 配布検証 ⭕️ [Pending]

2026-10-13、手動起動workerと基本IPC、GUI起動時の接続まで実装済み。[現在の対応範囲・通信契約](worker.md)を参照。2026-10-17までに更新・設定・隔離・出力等のIPCと、起動中GUIからの所有権交代を実装済み。workerはアプリ実行ファイルのheadless入口を共有し、別helperの同梱を不要にした。正式署名・公証版での入口・権限・更新検証は未完了。開発環境の機能検証と正式配布の検証を区別する。

- GUIなしのRust binaryを追加する。専用のデータ領域引数を持ち、エンジン・設定・履歴・診断・転送ジャーナルは既存形式を互換読み込みする。本番ではdebug用データパス環境変数を受け付けない。
- IPCはユーザー専用Unix domain socket。0700の親、0600のsocket、同じUIDのpeerを確認する。macOSのsocketパス長制限を生成時に検証し、長すぎるパスは診断する。公開TCPポートは作らない。
- バージョン付きの要求・応答、サイズ上限、タイムアウト、要求IDを設ける。許可コマンドはSnapshot取得、設定保存、検査・停止、定義更新、既存の隔離・復元・整合性確認に限定する。任意の実行ファイル・シェル・SQLは受け付けない。操作IDで再送による二重実行を防ぐ。
- ファイル内容はWebViewへ流さない。通知は状態・進捗・診断だけ。JSONの保存ダイアログはGUIが担当し、worker側でも既存の保存先保護を適用する。
- 有効化時はGUI処理完了→Core所有権解放→worker起動・接続確認の順とする。無効化時もworker停止・ロック解放を確認してからGUI Coreを作る。接続失敗時に別Coreを無条件起動しない。

## 3. 予約実行 — 🟢 [Done] / 登録 — 🟠 [Next]

2026-10-25、共有スケジューラー、日付別claim、1時間の開始枠、missed/DST/時計変更、結果の原子的保存と再起動時復旧、定義自動更新を実装。注入時計と実workerの専用フォルダ検査で確認した。タイムゾーンは現在のオフセットを含む説明ラベルで保存し、IANA識別子ではない。保存済み予約の状態・理由は3言語の設定画面と診断JSONへ反映する。LaunchAgent登録・解除は次の実装対象。

- LaunchAgentはworkerをログイン時に開始し、worker内の時計で既存の予約時刻を判定する。GUIのmaintenanceを同時に動かさない。異常終了時の再起動はlaunchdで抑制し、無限の短周期再起動を避ける。
- 予約履歴を専用テーブルへ移し、ローカル日付と予約種別を一意キーにする。時刻設定を変更しても同日に再実行しない。予定時刻・タイムゾーン・設定版・開始/終了時刻・状態・失敗理由を保存する。DBトランザクションでclaimし、再起動した実行中ジョブは中断として残す。自動再実行しない。
- 初期の対象は既存のquick検査。予約した時刻の1時間内だけ実行し、スリープからの復帰がそれを過ぎた場合はmissedとして記録する。電源オフ中の埋め合わせはしない。DSTの重複時刻は一日一回、存在しない時刻はmissed、日付が逆戻りしても同日履歴を維持する。これらは注入可能な時計でテストする。
- 定義なし・エンジンなし・更新制限・権限不足・他操作中を区別する。他操作中は時間枠内で待機し、期限後はmissedにする。検出ファイルの自動削除・自動隔離は追加しない。
- 安定した.app内のhelper絶対パスを登録する。開発用targetパスを本番登録しない。アプリ移動、バージョン更新、古いhelper、許可拒否を検出し、設定で修復・解除できるようにする。所有するLabelとplistだけを扱う。
- オフにすると新規ジョブを拒否し、実行中操作の安全な停止・子プロセス回収を待って登録を解除する。転送の確定処理中は完了を待つ。履歴・定義・隔離データは保持する。

[Appleの予約ジョブ資料](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/ScheduledJobs.html)では、StartCalendarIntervalのスリープ復帰時の実行と電源オフ時の欠落が説明されている。本計画は常駐workerの時計を使用するため、上記の実行枠とmissed判定をアプリ側で実装し、launchdの追いつき実行に依存しない。

## 4. clamd — 🔴 [Later]

- 別の任意設定で有効化する。workerが自分の子プロセスとして起動・監視・停止する。既存Homebrewのサービスや他アプリのclamdを停止しない。専用設定、私有定義、ユーザー専用Unix socketを使い、TCPは有効にしない。
- 起動時はPINGとVERSIONCOMMANDSを期限付きで確認する。状態はdisabled / starting / ready / reloading / stopping / failed。設定・エンジン変更は停止後に適用する。失敗理由と再試行期限を既存診断JSONへ含める。
- Rustで通常ファイルを安全に開き、INSTREAMで読み込んだデータをclamdへ送る。ハッシュと読取前後のメタデータで途中変更を拒否する。StreamMaxLength、MaxFileSize、MaxScanSize、アーカイブ等の制限を対応表で管理し、現在の設定と同等にできない場合は明示する。ERROR・制限超過・接続断・未対応命令をcleanと扱わない。
- 定義更新との排他を維持する。更新成功後のRELOAD応答だけで新定義の準備完了と判断せず、バージョンと検査可能状態を確認する。準備中は新規検査を待機する。
- 自動再起動は回数と待機時間を制限する。clamdが失敗した検査は未完了として残す。clamscanでの再検査は新しい操作として明示し、途中結果を成功扱いで継ぎ足さない。
- 停止は所有する子への終了要求、期限付き待機、必要時の強制終了と回収を行う。PIDファイルだけを信頼して他プロセスをkillしない。メモリ消費、起動時間、停止応答時間を実機測定して既定値を決める。

Unix socket、INSTREAMのサイズ制限、PING、VERSIONCOMMANDS、RELOADは[ClamAV公式プロトコル](https://docs.clamav.net/manual/Usage/ClamdProtocol.html)に従う。FILDESはビルド側の対応が必要なため初期実装に含めない。

## UI・権限・配布

設定に有効/無効、稼働状態、最終実行、次回予定、停止・再試行を置く。説明は情報アイコンのhover/focusに入れる。許可不足・失敗理由・期限は常時確認できる表示にする。英語・日本語・简体中文を同時に追加する。

helperにはGUIとは別にアクセス制限がかかる可能性がある。Full Disk Accessを継承すると仮定しない。権限の取得を装う処理や管理者昇格で回避せず、対象を開けなかった場合は未検査を保存する。サービス許可とファイルアクセス許可は別状態にする。

workerは.app内のメイン実行ファイルの`--worker`入口を使用する。更新時にIPC版と所有者の交代を確認する。正式配布はこの入口を含むアプリのDeveloper ID署名・公証を通す。アドホック署名での開発検証は正式配布検証の代替にしない。

## 完了条件

| 段階 | 開発環境の検証 | 実機・配布の確認 |
| --- | --- | --- |
| 共有コア | 既存回帰、プロセス間排他、クラッシュ後復旧 | ⭕️ [Pending] 外付け切断中の復旧 |
| worker / IPC | peer拒否、上限、切断・再接続、要求再送、GUI所有権交代 | ⭕️ [Pending] helper固有のファイル権限 |
| 予約 | 🟢 [Done] 注入時計、重複claim、DST、更新との競合、missed、GUIなし実worker検査 | ⭕️ [Pending] 正式配布版でのGUI終了、ログイン、スリープ復帰、許可拒否、macOS 12 / Intel |
| clamd | 模擬socket、分割応答、サイズ超過、切断、停止・reload失敗 | ⭕️ [Pending] 実エンジン、定義更新、メモリ、長時間稼働 |
| 配布 | 同梱パス・IPC互換性・解除対象の確認 | ⭕️ [Pending] 正式署名・公証、クリーンMacの登録・更新・解除 |

各段階の検証を完了してから次へ進む。未実施の実機確認をDoneへ変更しない。

English: Start with a shared Rust core and exclusive process ownership, then a per-user worker for scheduled clamscan jobs. Optional clamd support follows. The worker and daily scheduling are implemented; login service registration and signed-release validation remain pending.

简体中文：先分离共享 Rust 核心并实现进程间独占，再添加用户级后台进程和 clamscan 定时扫描，最后添加可选的 clamd。后台进程和每日定时扫描已实现；登录服务注册及正式签名发行版验证仍待完成。
