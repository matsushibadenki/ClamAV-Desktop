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

English: Software updates use signed GitHub archives with mandatory signed-version verification. The local notarization profile is `clamav-desktop`. GitHub Hosted CI needs its own release Secrets; it creates drafts only. Back up the ignored updater private key. End-to-end upgrades from a published newer version and Intel/macOS 12 validation are pending.

简体中文：软件更新使用 GitHub 的签名归档，并强制验证签名中的版本。本地公证配置名为 `clamav-desktop`。GitHub 托管 CI 需要单独设置发行 Secrets，流程只创建草稿。请备份已排除在 Git 外的更新私钥。发布新版本后的完整更新流程，以及 Intel/macOS 12 实机验证仍待完成。

## GitHub CIの確認結果（2026-10-14）

[実行38504789164](https://github.com/matsushibadenki/ClamAV-Desktop/actions/runs/38504789164)はbuild・draftとも成功。GitHub生成のDMGと更新アーカイブをダウンロードし、SHA-256、公開鍵による署名・署名内バージョン照合、改変拒否、公証チケットを再確認した。v0.2.0はユーザーのデプロイ指示により公開済み。GitHubによる空白のファイル名変換を避けるため、DMG名を`ClamAV-Desktop_0.2.0_aarch64.dmg`へ統一し、Draftのチェックサム記録も同期した。アプリ本体の内容は変更していない。

更新署名鍵はチャットへ提示されたため、鍵移行を推奨する。v0.2.0は現行公開鍵で公開済みのため、単純な鍵差し替えでは既存クライアントの更新が失敗する。互換性を保つ移行計画または再インストール案内を用意する。

[公開v0.2.0](https://github.com/matsushibadenki/ClamAV-Desktop/releases/tag/v0.2.0)の`latest.json`、DMG、更新アーカイブを認証なしで取得し、署名情報・SHA-256が検証済み成果物と一致することを確認した。より新しい版への実インストールと再起動は未検証。
