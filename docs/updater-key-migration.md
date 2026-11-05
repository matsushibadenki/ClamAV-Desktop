# 更新署名鍵の移行計画

2026-11-05更新。現段階は **🟢 [Done] 二系統配信ツール・新鍵のローカル生成と実署名検証**、**🟢 [Done] 外付け暗号化バックアップ・0.3.2への組込み・CI署名鍵更新**、**🟠 [Next] 署名・公証・公開**。製品はK1とlatest-v2.jsonへ切替済み。公開リリースは成果物検証後に更新する。

## 判断と対象

現行の更新鍵をK0、新しく生成する鍵をK1と呼ぶ。K0の秘密鍵は以前チャットに提示された記録がある。**基本方針は、署名・公証済みDMGを利用者が手動で導入し、K1を新しい信頼起点にする方法。** K0署名による自動中継は互換性をつなぐ手段にはなるが、K0の漏えいを取り消すことはできない。

Tauriはアプリに設定した公開鍵で更新署名を検証する。公開鍵だけをK1へ変更してK1署名の更新を従来のlatest.jsonへ出すと、K0を持つ既存クライアントは拒否する。公開鍵と更新先は、移行済みアプリに同時に組み込む。署名検証・署名内バージョン検証を省略する方法や、検証失敗後の旧鍵へのフォールバックは採用しない。

基礎仕様：[Tauri Updaterの署名と配信](https://v2.tauri.app/plugin/updater/)。以下の二系統配信は本プロジェクト向けの設計であり、Tauriが自動的に鍵を移行する機能ではない。

| 利用者の版 | 現在の経路 | 移行時の扱い |
| --- | --- | --- |
| 0.2.0 / 0.3.0 | K0、latest.json。更新確認UIに既知の問題あり | 初回は新しいDMGを手動導入。更新先だけでは旧UIを修復できない |
| 0.3.1 | K0、latest.json。更新・再起動の修正済み | 基本方針では新DMGからK1へ移行。任意の中継方式は下記の条件付き |
| 将来の移行版以降 | K1、latest-v2.json（予定） | 以後はK1署名を検証。K0を受理しない |

アプリ識別子とデータ保存先を維持し、設定・検査履歴・隔離データを引き継ぐ。移行版のバージョンは公開前に決める。以下のBは例示用の変数であり、特定バージョンの公開予約ではない。

## 1. 今回実装した配布前のチェック — 🟢 [Done]

`src-tauri/updater-trust.json`に公開0.3.1の公開鍵・従来の更新先を基準として保存した。秘密鍵は含まない。公開鍵の表示コメントではなく、アルゴリズム・鍵ID・鍵本体のパケット全体のSHA-256で照合する。同じ鍵IDを持つ異なる鍵も拒否する。

`node scripts/check-updater-trust.mjs`は以下を検証する。

- 公開鍵が基準と一致し、minisign公開鍵の形式が正しい。
- 更新先が既存のGitHub latest.json一つであり、別URLや追加フォールバックがない。
- `requireSignedVersion`が厳密にtrueである。
- バージョンが基準0.3.1以上の安定版である。
- 現行のschemaVersion=1 / legacy、または明示したschemaVersion=2 / manualを検証する。manualでは異なるK1、専用更新先、基準より新しい初回移行版、固定した旧フィードのSHA-256を必須とする。未実装のbridgeは拒否する。

正式配布スクリプトは署名資格情報・公証・ビルド出力に触れる前にこのチェックを行う。成果物収集とDraft作成にも同じチェックを置き、GitHub Actionsではキーチェーン設定前に実行する。設定と方針を意図的に同時変更するレビューまで防ぐ仕組みではなく、不用意な鍵・配信URL変更を検出するためのもの。製品内の署名検証は従来のRust実装を維持する。

## 2. 新しい信頼起点の準備 — 🟢 [Done] ローカル準備 / 🟠 [Next] 切替

1. K1を安全なローカル環境で生成する。秘密値を標準出力・チャット・ログに出さず、Git対象外の私有領域と別媒体のバックアップで保持する。所有者がバックアップからの復元を確認する。
2. K1の公開鍵の指紋をレビューし、K0と異なることを確認する。以後のリリースでK0の秘密鍵を自動選択しない。GitHub Secretの置換は署名照合と配信経路の準備後に行う。
3. 移行版BにK1と`https://github.com/matsushibadenki/ClamAV-Desktop/releases/latest/download/latest-v2.json`を組み込む。実装済みのschemaVersion=2 / manualを使用し、K0の基準情報を残したままsuccessorにK1・新URL・初回移行版を指定する。基準ファイルのK0だけをK1へ書き換える手順にはしない。
4. K1署名のアーカイブをK1で検証し、K0では拒否することを確認する。署名内の版不一致、改変、誤った秘密鍵も拒否する。
5. Developer ID署名・公証済みDMGから導入し、データ保持、再起動、K1で署名した次の版への実更新を確認する。手動導入時には同一Team ID・署名・公証を別途検証する。更新署名とAppleの署名は異なる検証である。

### 準備済みのK1とツール（2026-11-04）

K1の公開鍵パケットのSHA-256指紋は`a6e64b0f8d0128b4a2401a1162463eb4608c90ce512cf4e88ef41580ad1fbf99`。K0とは異なる。秘密値はこの文書・標準出力・ログに含めていない。

`node scripts/prepare-updater-key.mjs --generate --version X.Y.Z`は、Git対象外の`.secrets/updater-v2`を排他的に作成する。既存の鍵を上書き・再生成しない。秘密鍵とローカル復元コピーは0600、親フォルダは0700。生成CLIの出力は失敗時も転送しない。鍵ファイルの署名用パスワードは空で、私有ディレクトリとアクセス権で保護する。外部媒体には裸の鍵をコピーせず、下記の暗号化イメージを使用する。

新鍵とローカル復元コピーの両方で無害なプローブを署名し、Rustのminisign検証器で正しい鍵・版の成功、旧公開鍵・改変データ・異なる版の拒否を確認した。`node scripts/prepare-updater-key.mjs --verify`でディスクから読み直して再確認できる。プローブ・復元用の一時ファイルは私有領域で作成し、検証後に削除する。

0.4.0向けの`candidate-tauri.conf.json`と`candidate-updater-trust.json`も同じ私有領域に保存した。これはレビュー用候補であり、稼働中の製品設定ではない。適用時は公開鍵・更新先・版の差分を確認し、package.json、ロックファイル、Cargoの各版も同期する。設定ファイルを無検討で丸ごと上書きしない。

### 外付け暗号化バックアップと復元確認

```sh
node scripts/prepare-updater-key.mjs --backup '/absolute/path/on/another-volume'
```

Node.js 22以上とmacOSのローカルターミナルで実行する。別ファイルシステムの、所有者が指定した実在フォルダを保存先とする。16文字以上のASCIIパスフレーズを非表示で2回入力する。パスフレーズはチャットへ送らない。コードはパスフレーズを保存せず、引数・環境変数にも渡さない。バックアップを復旧するため、所有者はこのパスフレーズを別途安全に保持する必要がある。これはディスクイメージの復旧用であり、Tauri署名用のパスワードではない。

保存先には`ClamAV-updater-指紋先頭16文字.dmg`だけを書き込む。AES-256で暗号化し、非暗号化イメージを拒否する。鍵の平文ステージングと復元は内蔵の私有領域だけで行う。イメージを読取専用で開き、保存前と同じ鍵・公開鍵・設定候補であることを照合し、イメージから復元した鍵で実署名を再検証する。マウント解除・保存ファイルの同期後にだけ`preparation.json`へ独立バックアップ確認を記録する。

`--verify`は本体とローカル復元コピーの検査であり、外付け媒体の現在の存在を確認するものではない。外付けコピーを再検証するには、同じ保存先で`--backup`を再実行し、パスフレーズを入力する。以前に検証したイメージのハッシュと一致する場合だけ再利用する。未知・変更済み・未完了の既存イメージは上書きしない。解除に失敗したマウントには再帰削除を行わず、私有領域を保持して停止する。

今回指定された外付け媒体はexFATでGlobalPermissionsEnabled=falseだったため、個別権限に頼る裸の鍵コピーを行っていない。AES-256作成・読取専用マウント・復元照合・解除は、使い捨てデータとテスト用パスフレーズで実OS上の動作を確認済み。本番K1の外付け保存は所有者のローカル入力待ちで、現時点の記録はpending。ターミナルの自動操作はツールの安全制限で拒否されたため、所有者が上記コマンドを手動実行する。

## 3. 配信経路の分離 — 🟢 [Done] ツール / ⭕️ [Pending] 実配信

GitHubの`releases/latest/download/名前`は最新リリース内の同名ファイルへ解決される。新しいリリースを公開するたびに、必要な両系統のファイルを収録する。旧クライアントのURL自体は変更できない。

| ファイル | 基本方針の内容 | 検証する鍵 |
| --- | --- | --- |
| latest.json | 最後にK0で検証済みの0.3.1のメタデータを保持し、元の版・URL・署名を変更しない | K0 |
| latest-v2.json | K1で署名した新しい移行版B、以後は最新のK1版 | K1 |

K0版のフィードにK1署名を載せない。K1版のフィードにK0へのフォールバックを入れない。旧フィードを凍結すると0.3.1は旧系統内で「最新」と表示され、手動移行を遠隔で表示できない制約があるため、GitHubのリリース説明・配布ページに3言語の移行案内を置く。既存バイナリに遠隔で新しい案内UIを追加できると仮定しない。

`merge-updater.mjs`、`collect-release.mjs`、`draft-release.mjs`は二系統に対応済み。新系統の版を現在のアプリと照合し、旧系統は別途ハッシュと基準版を照合する。新フィードの署名とアーカイブの署名ファイル、URL、全アーカイブのSHA-256、SHA256SUMSを検証する。実際の暗号署名は従来どおり正式ビルド内のRust検証が担当する。Nodeの配布前検証は署名ファイルとフィードの整合性確認であり、暗号検証の代替ではない。

現行設定は引き続きlegacy。manualへの実切替には新しい鍵・版・配信先を製品へ組み込む作業が必要。0.4.0をローカルの設定候補として準備した。公開版の確定・予約を意味しない。

### 実装した配布規則

- `src-tauri/updater-legacy.json`は公開0.3.1のlatest.jsonそのもの。GitHubの公開asset digestと一致するSHA-256は`9ca21cf6aa0d1c71a8114276ff01b3014a82da76da1e54ce374fee88b9442e40`。manualのpolicy.legacyFeedSha256でこのバイト列を固定する。JSONを再整形して保存し直さない。
- manualではsuccessor.publicKeyがK0と異なること、successor.endpointがlatest-v2.jsonの固定URLであること、successor.firstVersionが基準0.3.1より新しく、現在版がその版以上であることを検証する。
- 新しいビルドのrelease.jsonにはモード・公開鍵指紋・基準版・旧フィードのハッシュをupdaterTrustとして記録する。manualの収集・Draft作成はこの記録を必須とし、現在の製品設定と一致しない古い成果物を拒否する。
- manualビルドは`latest-v2-aarch64.json`等を生成する。収集後、mergeが`latest-v2.json`を生成し、`latest.json`を固定済み旧フィードのまま保持する。legacyの従来形式も引き続き利用できる。
- manualビルドでは署名鍵の明示指定が必要。旧ローカル鍵`.secrets/updater.key`への自動フォールバックはしない。明示指定された鍵がK1であることは、ビルド時の実署名検証で確認する。GitHubの既存Secretが自動的にK1へ変わるわけではない。
- 全成果物を検証してから収集する。署名・ハッシュ・フィードが不整合なら収集を始めず、未知ファイルやリンクを含む保存先も拒否する。前版は別フォルダに保持する。新規公開対象は検証したファイルの一覧だけに限定し、中間のプラットフォーム別JSONや前版をアップロードしない。

移行時は新しいステージング領域を使う。以下はツールの実行順であり、現行legacy設定のまま新鍵版が作られるという意味ではない。

```sh
node scripts/check-updater-trust.mjs
npm run test:release
node scripts/collect-release.mjs release-output/migration
node scripts/merge-updater.mjs release-output/migration
node scripts/draft-release.mjs release-output/migration --check
```

Draft検証には`GITHUB_SHA`としてレビュー済みソースコミットの完全なSHAを渡す。`--check`はファイル一覧を表示するだけでGitHubへ接続・アップロードしない。実際のDraft作成では最後の`--check`を外す。公証・公開・CI Secret更新は、その時点の具体的な成果物と鍵の確認後に行う。

現時点の収集・Draft検証は単一アーキテクチャのリリースを対象とする。複数アーキテクチャの同時配布を検証済みと扱わない。

## 任意の自動中継方式 — 🔴 [Later]

手動移行の負担を減らす必要があり、K0への残存リスクを明示して採用する場合のみ検討する。

- 中継版Bの中身にK1と新フィードURLを組み込み、BのアーカイブはK0でも署名する。旧クライアントはBを検証・導入・再起動してからK1を使う。
- 同じBのアーカイブにK1の署名も別ファイルで付ける。latest.jsonはK0署名、latest-v2.jsonはK1署名を使用し、片方で上書きしない。
- 次の版CをK1で配布しても、latest.jsonはBのK0署名メタデータへ固定する。長く起動していなかった0.3.1もBを経由できるよう、Bのアーカイブと旧フィードを維持する。
- K0を持つ攻撃者が旧クライアント向け更新を偽造できる問題は残る。中継だけでK1の真正性を安全に再確立したと主張しない。K0漏えいへの復旧としては、署名・公証を独立確認できる手動DMGを優先する。

この方式の配布・旧鍵の再使用・新鍵のGitHub登録を今回承認済みと扱わない。

## 検証と中止条件

| 項目 | 状態 |
| --- | --- |
| 現行鍵・URL・署名内版検証の設定変更を拒否 | 🟢 [Done] Nodeの回帰・プロセステスト |
| 現行公開アーカイブの署名・改変拒否 | 🟢 [Done] 保存済み0.3.1のアーカイブでRust検証を実行 |
| 新鍵を使うプローブ署名・誤鍵・版偽装の拒否 | 🟢 [Done] K1本体・ローカル復元コピーの実署名で確認 |
| 本番K1の外付け暗号化バックアップ | ⭕️ [Pending] 所有者のパスフレーズ入力後に実コピー・復元検証 |
| 両フィードの保存・長期休止クライアントの経路 | 🟢 [Done] 固定バイト列を残し、次版でも新フィードだけが進むことをローカルで検証。実配信はPending |
| 新DMGからの導入、新鍵から次版への実更新 | ⭕️ [Pending] 新しい署名・公証済み配布物が必要 |
| Intel/macOS 12、ログインサービス・権限 | ⭕️ [Pending] 対象実機・サービスの検証が必要 |

署名・版・配信経路・設定/履歴/隔離データのいずれかの検証に失敗したら公開を止める。公開前は旧フィード・旧アーカイブを維持する。K1導入後の修正版もK1で署名し、K0へ戻すことを復旧手順にしない。秘密値を含まない版・指紋・成果物ハッシュ・実機結果のみを証跡へ保存する。

## English

The migration plan preserves installed-client compatibility while treating the exposed legacy key as an unsuitable basis for restoring trust. The default is a one-time manual installation of a Developer ID signed/notarized DMG containing a new key and a separate update feed. An optional legacy-signed bridge preserves availability, but does not undo legacy-key exposure. Release tooling now supports an explicit manual-migration policy, byte-pinned legacy feeds, a separate modern feed, build-trust receipts and a no-upload Draft check. K1 has now been generated locally and its signing/restoration probes pass. AES-256 external backup tooling is implemented and tested with disposable data. The production backup awaits the owner’s local hidden passphrase entry; production configuration, CI Secrets and the actual cutover remain unfinished.

## 简体中文

迁移计划兼顾现有客户端兼容性，但不把已暴露的旧密钥当作重新建立信任的可靠基础。默认方案是通过经过 Developer ID 签名及公证的 DMG 手动安装一次，引入新公钥与独立更新地址。旧密钥签名的中转版仅用于兼容性，无法消除旧密钥暴露的风险。发布工具现支持显式手动迁移策略、按字节固定的旧更新源、独立的新更新源、构建信任记录及不上传的草稿预检。K1 已在本地生成，并通过实际签名与恢复探针验证。AES-256 外置备份工具已实现，并使用一次性数据通过系统测试；生产密钥的外置备份等待所有者在本地隐藏输入口令。生产配置、CI Secret 及实际切换仍待完成。


## 0.3.2準備（2026-11-05）

ユーザー指定のパッチ更新でソース版を0.3.2へ同期した。0.4.0の私有候補は鍵準備時の例であり、公開予約ではない。独立バックアップ完了後に0.3.2を新しい信頼起点へ切り替える。現時点の公開版は0.3.1、外付けバックアップは未確認、製品の信頼設定とCI署名鍵は旧設定のまま。新規版の正式署名コマンドはlegacyモードのまま実行できない。

外付け媒体を接続し、次のコマンドでバックアップする。`--backup`はNode本体のオプションではなくスクリプト引数。npm経由なら引数の順を間違えずに実行できる。

```sh
npm run updater:backup -- '/absolute/path/on/another-volume'
```

保存先の媒体が未接続・別名のときは`backup-destination`工程で停止し、パスフレーズ入力もコピーも始めない。エラーは固定の工程名のみを表示し、子プロセスの秘密出力は引き続き抑制する。


## 独立バックアップ確認・切替（2026-11-05）

所有者がローカルで入力したパスフレーズによるAES-256イメージ作成、読取専用復元・実署名検証が成功した。別ファイルシステム上のイメージと記録済みSHA-256、暗号化属性を再照合した。K1の指紋は準備時と同じで、鍵は再生成していない。私有候補の0.4.0は準備プローブの版であり、正式な移行境界は0.3.2とする。

0.3.2のtauri.conf.jsonとtrust policyをmanualへ切替え、successor.firstVersion=0.3.2、配信先latest-v2.json、K1公開鍵を同期した。既存GitHubリポジトリSecret TAURI_SIGNING_PRIVATE_KEYも同じK1へ更新し、Environment releaseに旧鍵の上書きがないことを名前一覧だけで確認した。旧クライアントのlatest.jsonは固定済み0.3.1を保持する。初回DMG移行はREADMEと公開説明に英語・日本語・简体中文で記載する。

入力失敗時は固定の理由（短すぎる／不一致／ASCII以外／キャンセル）だけを表示する。入力値は出力せず、不正文字も改行まで非表示で消費してからエラーにする。私有鍵・パスフレーズ・バックアップイメージはGitHub成果物に含めない。
