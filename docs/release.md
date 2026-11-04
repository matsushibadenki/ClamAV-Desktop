# macOS release

## ローカルの正式配布

公証プロファイルは既定で`clamav-desktop`。指定済みのApple ID・Team ID・アプリ用パスワードはキーチェーンに保持し、ソースやログへ出力しない。Developer ID Applicationの有効な証明書を選ぶ。複数の証明書名が異なる場合は`APPLE_SIGNING_IDENTITY`で選択する。同じ名前の重複証明書はハッシュで指定して曖昧さを避ける。

```sh
npm ci
npm run release:mac -- --check
CARGO_BUILD_JOBS=2 npm run release:mac
node scripts/collect-release.mjs
node scripts/merge-updater.mjs release-output
```

`APPLE_KEYCHAIN_PROFILE`でプロファイル名を変更できる。`APPLE_NOTARY_KEYCHAIN`は専用キーチェーンを使用するCI向け。ローカルのApple資格情報を環境変数へ展開する必要はない。

公証申請の前にローカル成果物だけを準備する場合は`npm run release:mac -- --prepare-only`を使用する。テスト・Developer ID署名・署名検証まで実行し、公証用ZIPとSHA-256を出力して停止する。Appleへの公証申請・DMG生成・配布メタデータ生成・GitHub公開は行わない。その後、通常の`npm run release:mac`で公証を含む正式処理を行う。

処理順は、テスト→Developer ID署名→アプリの公証・staple・Gatekeeper確認→公証済みアプリを含むDMGの生成→DMGの公証・staple→更新アーカイブの生成・署名→更新署名と改変拒否の検証→SHA-256。公証に成功する前のアプリを更新アーカイブへ入れない。

出力は`src-tauri/target/release/bundle/`以下。`release-output/`には検証済みDMG、更新用`.app.tar.gz`、`.sig`、`latest.json`、SHA256SUMS、release.jsonだけを集める。現時点の正式ビルドはApple Silicon用。Intel・Universal・macOS 12実機は別途検証する。

## アプリのソフトウェアアップデート

設定の「ソフトウェアアップデート」で手動確認する。ダウンロード・検証・インストール・再起動はRust側で処理し、WebViewへファイル内容を渡さない。署名と署名に含まれるバージョンの照合を必須にし、リポジトリのHTTPS更新アーカイブだけを受け入れる。128MiBを超える更新は拒否する。ClamAV本体・ウイルス定義の更新とは別。

検査・転送・その他操作中のインストールは拒否する。更新インストール中は操作と終了を抑止し、完了後は再起動まで変更操作を拒否する。worker接続中のインストールは未対応で、worker停止後のアプリ再起動が必要。確認だけならworker接続中も可能。

配信URLは`https://github.com/matsushibadenki/ClamAV-Desktop/releases/latest/download/latest.json`。最初の公開リリースまでは更新情報を取得できない案内になる。0.1.0にはアップデーターがないため、最初の0.2.0はDMGから導入する。0.2.0以降のクライアントに向けて、より新しい署名済みリリースを公開する。

[Tauri公式アップデーター](https://v2.tauri.app/plugin/updater/)を使用する。公開鍵はtauri.conf.json、秘密鍵はGit対象外の`.secrets/updater.key`に0600、親は0700で保持する。**秘密鍵を安全にバックアップする。紛失すると既存クライアント向けの署名付き更新を作れない。** `TAURI_SIGNING_PRIVATE_KEY`（内容）または`TAURI_SIGNING_PRIVATE_KEY_PATH`（ファイル）、必要に応じて`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`を使う。秘密鍵をIssueやPRへ貼らない。

## 更新鍵・配信先の変更前の確認

[更新署名鍵の移行計画](updater-key-migration.md)に従う。`src-tauri/updater-trust.json`は公開0.3.1の信頼設定を固定した公開情報であり、秘密鍵ではない。

```sh
node scripts/check-updater-trust.mjs
npm run test:release
```

正式ビルドは資格情報・公証処理の前、収集・Draft作成は成果物処理の前に同じチェックを行う。GitHub Actionsもキーチェーン作成前に確認する。公開鍵・更新先URLの変更、追加のフォールバック、署名内バージョン検証の無効化、基準より古い版を拒否する。環境変数やCLIオプションでの無効化は用意しない。

新鍵への切替時は、製品設定だけでなく配信系統・成果物収集・Draft検証も合わせて実装する。現行はschemaVersion=1 / legacyを維持する。二系統配信ツールはschemaVersion=2 / manualにも対応し、K0の基準情報・固定旧フィード・successorのK1と新配信先を別々に検証する。bridgeは未対応。基準ファイルだけを新鍵へ書き換えて配布を通さない。旧鍵が共有された経緯から、K0署名の中継版だけで信頼を回復できると扱わず、署名・公証済みDMGで新鍵を導入する手順を優先する。

収集先は`node scripts/collect-release.mjs release-output/版名`で指定できる。前版を含む混在フォルダを再利用せず、移行版には新しい保存先を使う。`node scripts/draft-release.mjs 保存先 --check`は、`GITHUB_SHA`の形式と、公開対象のファイル一覧・版・署名ファイル・SHA-256・フィード・ビルドの信頼設定を確認し、指定したコミットを表示する。指定コミットがレビュー済みのビルド元であることは実行者が別途確認する。GitHubへのアップロードは行わない。

manualでは`latest-v2.json`と固定された旧`latest.json`の両方を配布する。新しい署名鍵は明示指定し、旧ローカル鍵へのフォールバックは拒否する。ビルド済み成果物の`updaterTrust`記録が現在の設定と一致することを必須にする。生成・収集・Draftの形式と移行手順は[計画書](updater-key-migration.md)を参照。現行の本番設定・公開フィードは切り替えていない。

K1のローカル準備ツールは`node scripts/prepare-updater-key.mjs --help`を参照。現時点では0.4.0向け鍵と設定候補を私有領域に準備済みで、実署名・ローカル復元を確認した。再生成せず`--verify`を使用する。外付け媒体への保存は`--backup 絶対パス`で暗号化イメージを作成し、所有者がローカルでパスフレーズを入力する。独立バックアップの成功記録を確認してから、候補の差分適用とCI Secretの置換へ進む。詳細と公開指紋は[鍵移行計画](updater-key-migration.md)に記録。

## GitHub Actionsの準備

`.github/workflows/release.yml`はmainブランチからの手動実行のみ。固定コミットのアクションを使い、署名済み成果物を作ってDraft Releaseに追加する。公開操作は含まない。ソースとワークフローをGitHubへ反映してから実行する。2026-10-14の再確認では、リポジトリSecret `CLAMAV_DESKTOP_KEY`の登録を確認。`APPLE_ID`・`APPLE_TEAM_ID`・`APPLE_PASSWORD`もリポジトリSecretsへ登録済み。`TAURI_SIGNING_PRIVATE_KEY`は検証済みローカル鍵との一致を確認して登録済み。`APPLE_CERTIFICATE`・`APPLE_CERTIFICATE_PASSWORD`の登録名も確認済みで、必要な6つのSecretは揃っている。Secretの値・p12の有効性はHosted CIで検証する。Hosted CIは2026-10-14に成功し、v0.2.0のDraft Releaseを作成済み。`CLAMAV_DESKTOP_KEY`は更新署名鍵とは別の認証情報との確認を受けており、用途が確定するまでワークフローでは参照しない。更新署名には別途登録済みの`TAURI_SIGNING_PRIVATE_KEY`を使用する。

以下のSecretはリポジトリまたはGitHubの`release` Environmentに登録する。Environmentの対象ブランチをmainに制限し、レビュー後の実行にする。

| Secret | 内容 |
| --- | --- |
| APPLE_CERTIFICATE | Developer ID Application証明書と秘密鍵を含むパスワード付きp12のbase64 |
| APPLE_CERTIFICATE_PASSWORD | p12のパスワード |
| APPLE_ID | 公証用Apple ID |
| APPLE_PASSWORD | アプリ用パスワード |
| APPLE_TEAM_ID | 公証用Team ID |
| TAURI_SIGNING_PRIVATE_KEY | ローカルの更新署名秘密鍵と同じ内容 |
| TAURI_SIGNING_PRIVATE_KEY_PASSWORD | 更新署名鍵のパスワード。現在のローカル鍵は空 |

ローカルの`store-credentials`設定はGitHub Hosted Runnerへ自動転送されない。CIは一時キーチェーンへp12を取り込み、上記資格情報から同名プロファイルを作る。終了時はキーチェーンと一時ファイルを削除する。証明書のエクスポート・Secretsの登録は所有者が行う。値をチャットやリポジトリへ保存しない。

必要な情報が不足した場合はビルド前に停止する。成功後のDraftはDMG、アーカイブ、署名、SHA-256、latest.jsonを確認してから公開する。同じタグのリリースは自動上書きしない。

## 開発用DMG

```sh
npm run dist:mac
```

ad-hoc署名。Appleおよび更新署名の秘密情報を環境から除外する。正式な署名・公証済み配布物ではない。GitHub配布用の収集処理は開発用成果物を拒否する。

## バージョン更新

package.json、package-lock.json、src-tauri/Cargo.toml、src-tauri/core/Cargo.toml、src-tauri/tauri.conf.jsonを同期する。配布手順は不一致を拒否する。依存関係はロックファイルを使用し、更新署名の公開鍵は既存クライアントの更新が必要な限り維持する。

English: Software updates use signed GitHub archives with mandatory signed-version verification. The local notarization profile is `clamav-desktop`. GitHub Hosted CI needs its own release Secrets; it creates drafts only. Back up the ignored updater private key. Actual download, replacement and automatic restart were validated with a repaired QA client updating to public 0.3.1. Original 0.2.0/0.3.0 clients require a one-time manual DMG migration. Intel/macOS 12 validation remains pending.

简体中文：软件更新使用 GitHub 的签名归档，并强制验证签名中的版本。本地公证配置名为 `clamav-desktop`。GitHub 托管 CI 需要单独设置发行 Secrets，流程只创建草稿。请备份已排除在 Git 外的更新私钥。已使用修复后的测试客户端验证实际下载、替换及自动重启至公开版 0.3.1。原始 0.2.0/0.3.0 客户端需要先手动安装一次 DMG。Intel/macOS 12 实机验证仍待完成。

## GitHub CIの確認結果（2026-10-14）

[実行38504789164](https://github.com/matsushibadenki/ClamAV-Desktop/actions/runs/38504789164)はbuild・draftとも成功。GitHub生成のDMGと更新アーカイブをダウンロードし、SHA-256、公開鍵による署名・署名内バージョン照合、改変拒否、公証チケットを再確認した。v0.2.0はユーザーのデプロイ指示により公開済み。GitHubによる空白のファイル名変換を避けるため、DMG名を`ClamAV-Desktop_0.2.0_aarch64.dmg`へ統一し、Draftのチェックサム記録も同期した。アプリ本体の内容は変更していない。

更新署名鍵はチャットへ提示されたため、鍵移行を推奨する。v0.2.0は現行公開鍵で公開済みのため、単純な鍵差し替えでは既存クライアントの更新が失敗する。互換性を保つ移行計画または再インストール案内を用意する。

[公開v0.2.0](https://github.com/matsushibadenki/ClamAV-Desktop/releases/tag/v0.2.0)の`latest.json`、DMG、更新アーカイブを認証なしで取得し、署名情報・SHA-256が検証済み成果物と一致することを確認した。当時は実インストールと再起動が未検証だった。2026-10-25の結果は下記を参照。


## 実更新で判明した互換性問題（2026-10-25）

公開0.2.0から公開0.3.0の検出までは成功したが、macOS WebViewで`window.confirm`の確認画面が表示されず、インストール命令へ到達しなかった。確認UIを修正した検証用クライアントでは実ダウンロードへ進んだが、0.3.0アーカイブの先頭にAppleDoubleの`._ClamAV Desktop.app`があり、Tauriの展開時に失敗した。アプリの置換前に失敗し、旧アプリと既存記録は保持された。

0.3.1では既存のModalを使った明示確認に変更し、アーカイブ生成時に`COPYFILE_DISABLE=1`を指定する。macOSの`tar -t`はAppleDoubleを隠すため、Python 3標準ライブラリのtarfileで実エントリーを検証し、単一.app以外・AppleDouble・不正パス・必須ファイル不足を配布前に拒否する。配布スクリプトにはPython 3も必要。

既にインストールされた0.2.0 / 0.3.0のUIは更新先アーカイブでは修復できない。利用者には修正版DMGからの一度の手動移行を案内する。公開済みアーカイブの差し替えや、署名・確認の迂回は行わない。更新公開鍵は維持し、鍵移行は別途互換性を保つ手順で行う。


### 0.3.1の公開と実更新結果

[公開0.3.1](https://github.com/matsushibadenki/ClamAV-Desktop/releases/tag/v0.3.1)は署名・公証・更新署名・改変拒否・実展開後の署名を確認済み。修正した検証用クライアント（表示版0.2.0、実際の配信URLと署名検証を使用）から公開アーカイブを取得し、置換・アプリ自身の再起動・0.3.1での更新確認まで成功した。UI再接続前にPID交代を観測し、テストツールによる再起動ではないことを確認した。元の公開0.2.0からの成功を意味しない。

既存の設定値・検査履歴・隔離記録・未完了転送を保持。旧設定に欠けていた保持期間30日・容量上限1024MiBは既存の互換既定値として保存された。検証は一時フォルダのアプリコピーで行い、`/Applications`のアプリは置き換えていない。詳しい証跡は[検証記録](verification.md)を参照。
