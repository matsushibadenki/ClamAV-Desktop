# Architecture

アプリ終了中の予約実行と任意のclamd対応は[背景サービスの実装計画](background-service-plan.md)を参照。共有処理はTauri非依存の`src-tauri/core` crateへ分離済み。通常はGUIプロセス内でCoreを所有する。[手動worker](worker.md)を先に起動した場合、GUIは起動時にIPCプロキシへ接続する。起動済みworkerはGUI終了後も予約検査・定義自動更新を行う。OSサービス登録は未実装。

workerは2スレッドのTokioで動作し、公開TCPポートを使わない。private socketの両側でOSのpeer UIDを確認する。GUIプロキシは最終受信状態の表示キャッシュだけを持ち、DB・取消フラグ・操作を所有しない。毎秒状態を取得し、切断時は3言語で通知して再接続を試みる。worker接続中はGUIのmaintenanceが処理をスキップする。Rust Runtimeの参照管理で起動中の所有権交代を行い、切断だけではCoreを追加生成しない。明示的なGUI復帰にもOSの排他取得を必要とする。未移行のコマンドはworker_operation_unavailableで拒否する。

Coreは`StateNotifier`を所有し、Snapshotを渡す。通知時はDocumentのMutexを解放しているため、通知先はCoreを再読取できる。GUIの`bridge.rs`だけがAppHandleとcore:stateイベントを扱う。検査タスクはArc<Core>を保持してTokio上で動き、ウインドウから独立している。Installerの画面起動とRendererはGUI側に残す。

Core生成時は通常ファイルの`owner.lock`をO_NOFOLLOW/O_CLOEXECで開き、所有UID、私有モード、ハードリンク数、パスとinodeの一致を確認してflockの非ブロッキング排他を取得する。SQLite初期化・中断検査の変更・転送復旧・保存用サブディレクトリの変更は取得後だけ行う。他プロセスの使用中はdata_in_useで拒否し、GUIは3言語の終了案内を表示する。ロックはCoreの存続中保持し、DBを閉じてからファイル記述子の解放で解除する。ロックファイルは削除しない。強制終了時もOSが解放する。既存のfile_gateは同一Core内の操作排他として残す。ロックに対応しない旧バージョンと同時起動しないことが必要。

`Core`がRustのDocument（Snapshot）、SQLite、取消フラグ、ファイル操作ロック、検証済みインストーラを所有します。React側はイベントを購読し、コマンドを送信します。各ウインドウにある状態はページやダイアログといった表示状態だけです。スキャン、設定、履歴、隔離、パネル配置はRustが所有します。

`Renderer`が操作ログのドッキング位置とTauriネイティブウインドウの作成・終了を扱います。メインと浮動ウインドウは同じSnapshotを使用します。ログパネルはボタンまたはドッキング領域へのドラッグで配置できます。重い画像処理や描画は現時点でなく、wgpuを追加する必要がありません。将来追加する場合もデータをRust/GPU側に保持し、IPCには命令と状態のみを流します。

ClamAVはシェルを介さず引数配列で起動します。出力とエラーを読み取り、停止・タイムアウト時に子プロセスを終了します。実際のファイル数と制限超過を表示し、推定パーセントは使いません。定義更新と検査は同時実行を防ぎます。

インストーラは固定の公式リリースと許可したHTTPSホストのみを使用します。Ciscoの公開鍵はソースに同梱し、PKGのPGP・Apple署名・公証を検証します。開く直前にもSHA-256を確認します。管理者パスワードはアプリでは扱わずmacOS Installerで入力します。

隔離は保存済み検出結果のみを受け付け、シンボリックリンクを拒否し、ハッシュを確認して`renamex_np(RENAME_EXCL)`で移動します。移動後に隔離データを再検査します。復元はハッシュと復元先を確認し、既存ファイルを上書きしません。SQLiteの操作ジャーナルで中断を認識します。別ボリュームではstatvfsのユーザー利用可能容量を確認し、ファイルサイズに1MiBの余裕を要求します。元ファイルを保持したままコピーを同期・照合・再検査し、その後に専用一時領域へ元ファイルを待避して再度照合します。コピー途中のENOSPCも容量不足として通知し、未確定転送を巻き戻します。隔離行がコミットされるまで元のデータを削除しません。一時ディレクトリの実効モードと所有者を確認します。例外はstatfsで判定したexFATの隔離元と復元先のみです。exFATの元ファイル待避は私有領域ではなく、完了まで隔離済みとは表示しません。隔離先の私有権限要件は緩和しません。exFATへの復元コピーはユーザーが指定した公開先に作成されます。exFATでディレクトリfsyncがEINVAL/ENOTSUPの場合はファイル同期とジャーナルで復旧しますが、強制切断時の耐久性は実機確認が必要です。SQLiteはWALとsynchronous=FULLを使用します。起動時は確定済み転送を後処理し、未確定転送を上書きなしで巻き戻します。接続・競合・整合性に問題があればデータを保持し、復旧待ちとして明示します。

WebViewにはshell/filesystemプラグインを公開していません。主要な変更コマンドはメインウインドウに限定しています。アプリのデータ領域はユーザー別で、ディレクトリは0700、隔離ファイルは0600です。権限・プロセス・ネットワークの詳細はRust側にあります。

ダウンロード容量管理はRustの専用モジュールで処理します。アプリのdownloads直下にあるUUID名のディレクトリだけを対象にし、通常ファイルのClamAV PKG/署名のみを含む保存先を認識します。ディレクトリとファイルの最新更新時刻から30日を経過したものだけを削除し、準備済みPKGの保存先は保持します。不明項目とシンボリックリンクは保持します。操作中の削除を拒否し、削除時に条件を再取得します。自動削除は行いません。

隔離の詳細取得もRustで行い、ファイル操作ゲートで転送と競合しないようにします。SQLiteの転送ジャーナルは所有する隔離IDで限定して返します。ファイルの存在・容量は読取時点のメタデータであり、保存済みSHA-256の再検証結果ではありません。リンクは追跡しません。WebViewは選択中の詳細ダイアログと表示フィルターのみを持ちます。

保持期間は1〜365日、ダウンロード容量上限は257〜16,384MiBをSQLiteの設定に保存します。旧設定には30日・1024MiBを補います。上限は管理対象のPKG/署名に適用し、新規取得前に最大256MiBのPKGと16KiBの署名の余裕、および転送先の空き容量を確認します。設定の縮小時も既存ファイルは削除しません。削除時には保存済み保持期間を再取得します。
隔離の検索はNFKC正規化・小文字化したパス、検出名、表示言語の状態名を対象にし、空白で区切った語をすべて満たす記録を表示します。検索と並び替えは共有スナップショットを変更せず、ウインドウ固有の表示条件として保持します。

隔離データの手動確認はRustのファイル操作ゲート内で実行します。パスのリンクを拒否し、O_NOFOLLOW/O_NONBLOCKで通常ファイルを開き、64KiB単位でSHA-256を算出します。読み取り前後のサイズ・変更時刻・inodeを確認し、途中変更を確認結果として受け入れません。最終確認日時・状態・実測ハッシュ・エラーをruntime_kvに保存します。この操作はファイルや隔離行の状態を変更せず、転送のコミット判定にも影響しません。復元時の独立したハッシュ照合は継続します。
未完了転送のジャーナルにはlast_errorとattempted_atを追加し、旧ジャーナルには未記録の既定値を適用します。復旧が失敗したときに理由を保存し、所有する隔離記録の詳細にのみ返します。復旧完了時は従来どおりジャーナルを削除します。診断情報は詳細画面の開閉項目に表示します。

整合性確認は共有Operationに対象ID、処理段階、確認済み容量、総容量を保存します。64KiBの読取ごとに専用AtomicBoolを確認し、約150ms間隔でWebViewへ状態だけを通知します。通常スキャンの停止フラグとは独立しています。結果保存へ移る直前に停止状態を確定し、その後の停止操作では完了結果を変更しません。停止した結果はcancelledとして保存し、actualDigestはnullにします。ファイル操作ゲートの競合時はbusyを返します。詳細を閉じてもメイン画面から停止でき、完了時に閉じたダイアログを再び開きません。
隔離JSON出力はschemaVersion=1、exportedAt、recordsを持ちます。全記録のDetailを含み、検索条件は適用しません。転送・整合性確認と競合しないタイミングでメタデータと最終結果を取得し、ファイルの再ハッシュはしません。通常ファイルの出力先を選び、0600の一時ファイルを同期してrenameで配置します。アプリ保存領域と記録にある元ファイルへの出力を拒否します。

診断の直近結果はengine_inspection・engine_install・engine_launch・definitions_updateの4種類に限定してruntime_kvへ保存します。各種類の成功で同じ種類の失敗を置き換えます。診断文は8192文字までとし、DB保存が失敗した場合でも起動中の共有状態に診断を残します。FreshClam出力は複数行を容量制限付きで保持し、終了コード0でも待機案内があれば更新成功にしません。
FreshClamのuntil after形式の時刻はローカル時刻として読み取り、未来の期限をUTCへ変換します。期限不明の制限時はアプリ側で24時間の待機期限を設定します。Rustが再試行を拒否し、UIも更新・再取得を無効化します。表示期限は1分以内のタイマーで更新します。署名・Gatekeeperの検証は従来どおり維持します。
診断JSONは形式バージョン・出力日時・アプリバージョン・OS/CPUと、共有状態、隔離詳細、キャッシュ情報、取得エラーを含みます。ファイルの内容や環境変数を収集しません。パス・ハッシュ・ログを含むため、出力画面で共有前の確認を案内します。保存先の保護と0600の一時ファイルによる配置は隔離JSONと共通です。

アプリ更新はGUIプロセスのRust Serviceが所有する。チェック・取得・インストール・再起動の状態と、検証済み更新候補をRustに保持し、WebViewへはメタデータと進捗だけを通知する。Tauri公式プラグイン2.13.3に固定し、requireSignedVersionを有効にする。GitHubの当該リポジトリのHTTPSアーカイブ以外は拒否する。取得中の容量確認と取得後の128MiB確認を行い、署名・バージョン検証に成功したデータだけをinstallへ渡す。Coreのファイル操作ロックと共有Operationを使って検査・転送と競合しないようにし、成功時は再起動待ちを保持する。worker接続時のインストールは拒否する。診断JSONにはappUpdate状態を追加する。

Settings writes are centralized in core/settings.rs. A Core-owned settings mutex orders SQLite commits and Document updates across concurrent clients; failed commits do not update Document. Both Tauri and worker IPC use this path. The additive save_settings command participates in the existing persistent request ledger, preserving at-most-once replay behavior.

Worker definition updates reserve the shared operation before accepting a request, then run as a worker-owned task outside the IPC command mutex. The persistent request ledger records acceptance or rejection, not eventual completion; snapshots and diagnostics carry progress and outcome. A separate definition cancellation flag prevents scan-stop requests from cancelling updates. Shutdown cancels and joins the update task before releasing Core ownership.

Integrity IPC uses an acceptance handshake from the blocking verifier after operation/file-lock acquisition. The worker retains its task through GUI disconnects, and caches up to 64 request-correlated completion results separately from the persistent command ledger. Tauri waits via short read-only result requests and propagates save errors while preserving the existing Detail return type. Shutdown requests cancellation and joins the blocking task without releasing Core early; stalled OS I/O can delay shutdown.

Quarantine/restore/recovery share a Core operation wrapper in both local and worker modes. It acquires the file gate without queuing, reserves Document.operation under the document mutex, and clears/publishes the operation on normal completion or error. Worker file jobs use the same acceptance handshake and bounded result-cache pattern as integrity jobs; the existing durable command ledger prevents repeated mutations. Accepted file jobs are not cancelled at shutdown, and Core ownership is retained until they finish.

History/quarantine/diagnostics exports and cache cleanup use the same file-job lifecycle. Core exports::Kind selects the schema; serialization and atomic private-file writing stay in the owning process. File-gate-held internal helpers avoid nested locking during report collection and output. Cache reads are offloaded to blocking tasks and do not consume the mutation ledger. GUI diagnostic metadata is bounded and optional; report bytes never cross IPC or WebView.

Manual engine inspection reserves Document.operation, runs outside the IPC mutation mutex, and exposes bounded request-correlated Engine results. GUI and local modes share begin/finish handling. Core Ownership records its acquiring PID and explicitly LOCK_UNs only from that PID after other Core fields are dropped; CLOEXEC is retained. This prevents close-only release from being delayed by fork-inherited descriptors prior to exec without weakening active-owner exclusion.

Live handoff is coordinated by `runtime.rs`. Backend leases and Core strong references prevent teardown while commands, scans, installer, updater or maintenance retain state. The GUI launches its own executable with a fixed `--worker` entry before Tauri initialization, avoiding a separately bundled helper. IPC `shutdown_if_idle` checks work and sets stopping under the mutation gate. The GUI acquires the OS lock before recreating Core; failed or timed-out startup cannot bypass ownership. GUI renderer placement is restored, while transient activity logs restart with the new owner. Production signature/notarization and service registration remain separate verification steps.

予約と定義自動更新は共有コアのschedulerが判定し、GUIとworkerのどちらか一方の所有者が30秒ごとに呼び出す。Coreのmaintenance_gateと設定ゲート、workerのコマンドゲートで開始と終了・設定変更を調整する。日付と種別で一意のscheduled_runsに開始前claimを保存し、検査結果を既存scansと同じSQLiteトランザクションで確定する。Core初期化時は検査履歴を復旧してから予約結果を復旧する。Snapshotは最新予約一件、診断JSONは全予約記録を返す。実行時間枠・時計変更・再実行抑止の契約は[workerの予約実行](worker.md#予約実行2026-10-25)を参照。
