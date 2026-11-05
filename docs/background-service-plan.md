# Background service implementation plan

共有コア・worker・予約実行と、LaunchAgentの事前検証・明示的な登録解除CLIは実装済み。GUI設定・所有権交代も実装済み。OS許可の照会と既知の移動登録修復も実装済み。正式配布版でのOS登録・許可・移動修復検証は未完了で、以下は残作業を含む設計・検証計画。

## 優先順位と構成

最初は既存のclamscanでアプリ終了中の予約検査を実現する。clamdは起動時間の短縮を目的とした任意の後続機能にする。常駐エンジンのメモリ負担を予約検査の必須条件にしない。

Rustの共有ライブラリがDocument、SQLite、検査、定義更新、隔離を所有する。Tauriは表示・ダイアログ・Rendererを担当する。バックグラウンド有効時は同じユーザーのRust workerが唯一のDocument所有者となり、TauriはローカルIPC経由で命令とSnapshotを取得する。各WebViewは表示状態だけを持つ。無効時は従来のGUI内Coreを使うが、所有権を取得するまでDBの初期化・復旧・変更を開始しない。

初期対象はログイン中のユーザー別LaunchAgent。管理者権限を必要とするLaunchDaemon、ログアウト中の検査、スリープ解除は対象に含めない。バックグラウンド有効化は既定でオフにする。macOS 12のサポートを維持するため、初期登録アダプターはユーザーのLaunchAgentsとlaunchctlを使用し、OS呼出しを共有コアから分離する。登録・解除はシェルを介さない引数配列で実装する。配布前にOS別の登録・許可状態を実機確認する。

## 1. 共有コアと所有権 — 🟢 [Done]

2026-10-13実装済み。`src-tauri/core`はTauriに依存しない独立crate。Coreが通知先の`StateNotifier`を持ち、GUIの`bridge.rs`がイベントへ変換する。`owner.lock`のOS排他をDB初期化・復旧前に取得する。別プロセスの競合、強制終了後の取得、一度だけの復旧、不正なロック、再入可能な通知をテストした。実ClamAVの無害な独自シグネチャでGUIなしの検査・隔離・整合性確認・復元も確認した。workerは次項のとおり実装済み。LaunchAgentアダプターは2026-11-04に追加し、実登録は配布検証待ち。

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

## 3. 予約実行・登録CLI — 🟢 [Done] / GUI統合 — 🟠 [Next]

2026-10-25、共有スケジューラー、日付別claim、1時間の開始枠、missed/DST/時計変更、結果の原子的保存と再起動時復旧、定義自動更新を実装。注入時計と実workerの専用フォルダ検査で確認した。タイムゾーンは現在のオフセットを含む説明ラベルで保存し、IANA識別子ではない。保存済み予約の状態・理由は3言語の設定画面と診断JSONへ反映する。LaunchAgentの事前検証は2026-11-04に実装。同日に登録・解除の専用CLIも追加。GUI所有権交代との統合も同日に実装。OS許可・読み込み状態と既知の移動登録修復も同日に実装。実配布での許可変更・再接続の動作確認が残る。

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

English: Start with a shared Rust core and exclusive process ownership, then a per-user worker for scheduled clamscan jobs. Optional clamd support follows. The worker, daily scheduling and explicit login-service CLI are implemented; OS permission queries and signed-release validation remain pending.

简体中文：先分离共享 Rust 核心并实现进程间独占，再添加用户级后台进程和 clamscan 定时扫描，最后添加可选的 clamd。后台进程、每日定时扫描和显式登录服务命令已实现；系统许可查询及正式签名发行版验证仍待完成。


## LaunchAgent事前検証（2026-11-04）— 🟢 [Done]

共有コアの`login_service`は、登録前の読み取りだけを行う。アプリ実行ファイルの`--login-service-check`入口はTauri初期化前にJSONを出力する。引数の追加・パス上書きは受け付けず、ホームは`HOME`環境変数ではなくOSのアカウント情報から取得する。

受入先は`/Applications/ClamAV Desktop.app`またはアカウントの`~/Applications/ClamAV Desktop.app`内の固定実行ファイルのみ。各パス要素の所有者・書込み権限・リンクを検証する。標準のroot:admin所有`/Applications`だけは管理者グループの書込みを許容する。Info.plistの識別子・実行ファイル名・APPL種別と、Developer ID Application要件付きcodesign検証も必須。開発target、一時展開、移動先の任意フォルダ、リンク経由の配置は拒否する。これは登録候補の検証であり、特定Team IDの固定やGatekeeper公証判定の代替ではない。

保存先はアカウントの`Library/Application Support/app.clamav.desktop`に固定し、既存の本人所有0700ディレクトリを要求する。登録候補は本人の`Library/LaunchAgents/app.clamav.desktop.worker.plist`のみ。既存登録は本人所有・単一リンク・非公開の通常ファイルを上限16KiBで読み、完全一致／競合／未作成に分ける。不一致や旧パスのファイルは自動で修正・削除しない。事前検証はディレクトリも作成しない。

plistは引数配列で固定worker入口を指定し、XML文字をエスケープする。ログイン時起動、異常終了時のみ再起動、最短60秒間隔、077のumaskを指定する。日付ごとの実行判定は既存のRustスケジューラーに任せる。構成キーは[Appleのlaunchd.plist仕様](https://github.com/apple-oss-distributions/launchd/blob/main/man/launchd.plist.5)を参照した。

`registration=matches`はファイル一致のみを表し、許可済み・launchd登録済み・worker稼働中を意味しない。`permission=unknown`、`loaded=null`を返し、後続のOS状態確認と分離する。登録・解除と終了待機は下記の専用CLIで実装。起動済みworkerのGUI引継ぎ、更新保護、3言語の設定UIも実装済み。OS許可・読み込み状態の照会も下記のとおり実装。ログイン・再起動・許可拒否の実機確認は⭕️ [Pending]。

English: Read-only LaunchAgent preflight validates the installed bundle, Developer ID signature, private data directory and exact registration bytes, then emits a plist candidate. It does not register a service or infer OS permission/running state. Registration, ownership handoff and settings UI remain next.

简体中文：只读 LaunchAgent 预检现可验证已安装应用、Developer ID 签名、私有数据目录及注册文件内容，并生成 plist 候选。预检不注册服务，也不推断系统许可或运行状态。服务注册、所有权移交和设置界面是下一步。


## LaunchAgent登録・解除CLI（2026-11-04）— 🟢 [Done] / 実登録 ⭕️ [Pending]

`--login-service enable|disable`はTauri起動前に共有コアへ分岐する。引数の追加、実行パス・UID・データパス・Labelの指定は受け付けず、毎回、前項の配布アプリ検証を行う。既定では登録しない。アプリの通常起動・予約設定の保存・事前確認だけではOS登録を変更しない。

管理操作には専用の`login-service.lock`を使用し、通常のCoreの`owner.lock`と分離する。登録側は既存GUI／手動workerの所有権を取得できなければ、停止要求を送らず`data_in_use`で拒否する。登録ファイルは0600の一時ファイルへ書込み・同期後、ハードリンクで上書きせず公開し、一時名を除去して親を同期する。既知の完全一致ファイルだけを再利用する。検証後に自分の固定Labelを`launchctl enable`し、Coreのロックを解放してから`bootstrap`する。15秒以内に認証済みIPCのSnapshotを受信して完了とする。失敗時も既存サービスを停止する巻き戻しは行わず、既知の登録を保持してエラーを返す。

解除側は登録が完全一致することを最初に確認し、Coreが使用中ならworkerへ`ShutdownIfIdle`だけを送る。検査・転送等の実行中は`busy`で拒否する。ackだけで解除せず、所有権の取得を最大15秒待つ。取得したロックを保持したまま固定のユーザーサービスだけを`bootout`し、再照合して登録ファイルを削除・同期する。履歴・定義・隔離データには触れない。登録が不明／移動済み／リンクならworkerへ停止要求も送らない。

すでにアンロード済みの場合は、検証したmacOSの「サービス不在」終了コード113に加え、GUIドメインの照会成功と同じサービスの不在結果を要求する。その他の失敗・曖昧な結果では登録ファイルを保持する。`launchctl print`の人間向け本文は解析しない。`launchctl`自体は20秒で制限し、タイムアウト時は子の停止・回収を試みる。未知のLabel、systemドメイン、sudo、強制kickstartは使用しない。現在のOSの仕様はローカル`launchctl(1)`のbootstrap/bootout/enable/printを確認した。

再実行で`bootstrap`が失敗した場合、ファイルが一致していても読み込み済みの定義が同一とは推測しない。GUIを閉じた状態で明示的に解除してから登録し直す。アプリ移動後に不一致となった登録の修復は未実装で、勝手に置換しない。署名済み新ビルドの実登録、macOS 12での終了コード、ログイン・許可拒否・再起動は⭕️ [Pending]。GUI内有効化は下記の所有権交代を経由する。公開済み0.3.1には未収録。

English: Explicit headless registration/removal is implemented. Registration refuses an existing data owner; removal asks the real worker to stop only when idle, then waits for exclusive ownership before unloading. Unknown files and OS failures preserve registration. OS calls are mocked in lifecycle tests; real worker IPC covers busy refusal and idle shutdown. Signed-app registration and GUI integration remain unfinished.

简体中文：已实现显式的无界面注册与移除。注册时若已有数据所有者则拒绝；移除时只在后台进程空闲时请求退出，并取得独占锁后再卸载。未知文件和系统调用失败时保留注册。生命周期测试使用模拟系统适配器，并通过真实 IPC 验证忙碌拒绝和空闲退出。正式签名应用的实际注册及 GUI 集成仍待完成。


## GUI設定・所有権交代（2026-11-04）— 🟢 [Done]

設定の実行プロセス欄へ、ログイン起動の有効化・登録解除・再確認を追加した。登録ファイルの未作成／一致／競合と、現在のGUI／worker実行を分ける。利用不可・登録済みの注意は常時表示し、動作説明は情報アイコンのhover/focusへ置く。英語・日本語・简体中文に対応する。登録は保存済み設定を使うため、未保存の予約変更は先に保存する。

RustのRuntimeは既存の切替ゲートとBackend参照数・処理中判定を共有する。配布パス・署名・通常データ領域をCore解放前に確認し、競合した登録や重複有効化も停止要求の前に拒否する。GUIから有効化する場合はCoreを破棄してから既存アダプターへ渡し、手動workerから有効化する場合はアイドル終了と所有権解放を待ってから登録する。成功後は認証済みSnapshotを取得してworkerプロキシへ切り替える。解除側はアダプターの安全な終了・登録解除を待ってからGUI Coreを再生成する。

管理処理はWebViewから独立したタスクで完了させ、切替中の通常終了を既存の終了ゲートで防ぐ。失敗時はOSロックを取得できる場合だけGUIへ戻し、取得不能ならworkerプロキシを維持する。別Coreを無条件に開かない。設定はSQLiteから復元し、ログパネル配置は前所有者のSnapshotから引き継ぐ。ローカル復帰後はエンジン状態を再確認する。

登録ファイルが存在する間は手動の実行プロセス切替とアプリ更新インストールをRust側でも拒否する。GUIがローカル所有へ復帰した部分失敗時も、この保護を維持する。先にログイン起動登録を解除する。登録済み表示はOSの許可・読み込み・次回ログインでの起動を保証せず、未確認と表示する。移動済み登録の修復とOS許可照会は下記のとおり実装。

実workerプロセスを使ってGUI→worker→GUI、手動worker→登録処理、処理中／参照保持の拒否、登録失敗からのGUI復帰、解除失敗時の生存所有者保護、設定・パネル保持を検証した。OS登録部分は模擬し、実ログイン設定は変更していない。署名済み.appでのGUI有効化・解除、許可拒否、ログイン・再起動は⭕️ [Pending]。

English: Settings now enables/removes login startup through Rust-owned handoff. The GUI releases Core before registration, stops an idle manual worker when necessary, adopts the authenticated worker, and reacquires Core only after safe removal. Failed operations preserve exclusive ownership. Manual switching and app installation are blocked while registration exists. OS permission and signed-app lifecycle validation remain pending.

简体中文：设置现可通过 Rust 所有权移交启用或移除登录启动。注册前释放 GUI Core，必要时等待空闲手动后台进程退出，然后连接已认证后台进程；安全移除后才重新取得 Core。失败时仍遵守独占锁。注册存在时禁止手动切换及应用更新安装。系统许可和正式签名应用的生命周期仍待验证。


## OS状態と移動登録修復（2026-11-04）— 🟢 [Done] / 実配布検証 ⭕️ [Pending]

許可はmacOS 13以降の`SMAppService.statusForLegacyPlist(at:)`（Objective-Cセレクター`statusForLegacyURL:`）を使う。[Appleの旧方式helper向け資料](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos)とSDKのServiceManagement/SMAppService.hを確認した。ServiceManagementをプロセス内で動的に読み込み、クラスとメソッドの存在を確認して呼ぶ。macOS 12は新APIを直接リンク・呼出しせず`unknown`とする。状態は`enabled`／`requires_approval`／`not_registered`／`not_found`／`unknown`。認識済みの一致・移動登録だけを照会し、不明な登録の許可を現在のアプリの許可として扱わない。`permissionSupported`はこの照会でAPIから結果を得られたことを示す。

読み込みは自分のGUIドメイン・固定Labelへの`launchctl print`の終了結果だけで確認する。成功なら`loaded=true`。終了113に加えGUIドメイン照会が成功したときだけ`false`。それ以外は`null`と`queryFailed=true`を返す。本文を解析せず、OS許可、読み込み、現在の実行プロセス、ファイルアクセス権限を分ける。読み込み済みは定義の存在であり、workerの稼働を保証しない。許可が必要なら公式`openSystemSettingsLoginItems`でログイン項目を開ける。許可をプログラムから変更・回避しない。

修復可能なのは、既存の生成テンプレート全体に一致する私有0600・単一リンクの登録だけ。違いは実行ファイルの位置だけとし、旧パスは`/Applications/ClamAV Desktop.app`と本人の`~/Applications/ClamAV Desktop.app`のもう一方に限定する。データ領域、Label、引数、再起動条件等が違えば修復しない。旧実行ファイルが存在する場合、旧位置がリンクの場合、読取不能・不明な形式の場合も競合扱い。現在のアプリは従来どおり安定パスとDeveloper ID署名を検証する。任意の移動先や名称変更には対応しない。

GUIの「登録を修復」とCLIの`--login-service repair`は同じRust経路を使う。操作を直列化し、workerのアイドル終了とCoreの排他取得を待って旧サービスを外す。旧登録を同じディレクトリの0600 `.backup`へ同期保存してから、新しい登録を一時ファイルへ書き、直前の再照合後に同一ディレクトリ内renameで置換する。新定義を起動して認証済みSnapshotを受け取った後、バックアップを削除・親を同期する。途中失敗時は旧登録またはそのバックアップを保持し、未知の内容を上書きしない。変更済み新登録は自動で旧パスへ戻さない。履歴・隔離・定義は保持する。

切断した旧workerからの修復も、キャッシュ表示を根拠にロックを奪わず、共有アダプターが所有権とアイドル停止を確認する。稼働中操作、旧アプリの再出現、登録内容の変化は修復を拒否する。OS許可変更後は再確認し、起動しなければ登録解除→有効化で明示的に再試行する。実配布環境での承認前後の起動・再接続、移動修復、macOS 12 / Intelは⭕️ [Pending]。

English: Official legacy-service authorization on macOS 13+ and conservative launchd loading queries are now separate states. A known, private registration pointing to a missing app in the other supported Applications location can be repaired with idle shutdown, ownership, backup and atomic replacement. Unknown templates, surviving old copies and links are refused. OS operations are mocked in repair tests; signed-app approval/move/login validation remains pending.

简体中文：macOS 13 及以上现使用官方 API 查询旧式服务许可，并单独查询 launchd 加载状态。仅修复指向另一个受支持 Applications 位置中已不存在旧应用的已知私有注册；通过空闲退出、独占锁、备份和原子替换保护数据。不修改未知模板、仍存在的旧副本或链接。修复测试使用模拟系统操作，正式签名应用的批准、移动和登录验证仍待完成。

## 許可変更後の復旧（0.3.2、2026-11-05）— 🟢 [Done]

GUIへ戻ったときの状態照会を重複抑止し、参照が解放されたコンポーネントへ更新を返さない。照会とプロセス変更を分け、OS許可を確認しただけでworkerを再起動しない。利用者の「再接続・再起動」またはCLI `recover`を契機にする。

登録が完全一致し、workerが応答する場合は検査中でも接続を維持し、停止・bootout・Core再生成を行わない。接続できなければ許可待ちを拒否し、アイドル終了と所有権取得を確認する。読み込み済みなら既知サービスを安全に外して読み直し、未読み込みならbootstrapする。不明な状態は変更しない。launchctl enableの上書きや強制kickstartは使わず、OSの許可を回避しない。起動後の認証済みSnapshotで完了とする。

失敗時も登録と保存データは保持し、ローカル所有権を取れる場合だけGUIへ戻す。生存所有者がいればプロキシを維持する。許可変更後の実配布検証はPendingとして残す。0.3.2のソース版を同期し、旧署名鍵で新規配布しないゲートを追加した。新鍵バックアップが未確認の間、配布アプリは公開しない。
