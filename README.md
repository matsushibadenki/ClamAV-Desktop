# ClamAV Desktop

macOS用のClamAVフロントエンド。Tauri 2とRustで処理・状態を管理し、ReactのWebViewで操作します。表示言語は日本語、English、简体中文です。

## 起動

必要環境：macOS 12以降、Xcode Command Line Tools、Node.js 20.19以降または22.12以降、Rust 1.88以降。

```sh
npm ci
npm run tauri dev
```

開発用アプリを作成するには：

```sh
npm run tauri -- build --debug --bundles app
```

出力：`src-tauri/target/debug/bundle/macos/ClamAV Desktop.app`。このアプリ自体の配布署名・公証はまだ行っていません。

## 使い方

1. 既存の公式PKG版またはHomebrew版ClamAVを自動検出します。未導入の場合は「エンジンを準備」から公式PKGをダウンロードします。固定したCisco公開鍵でPGP署名を検証し、Appleの署名・公証確認を通過したものだけをmacOS Installerで開きます。管理者認証とインストール完了はmacOSの画面で操作してください。
2. 定義がなければ「定義ファイルを更新」。FreshClamはアプリ専用のデータ領域に書き込みます。既存のHomebrew定義は読み取りにのみ使用します。
3. クイック検査はDownloads、フル検査はアクセス可能なホーム領域、カスタム検査は選択したフォルダを対象にします。全ディスクアクセスが必要な場合は設定画面からmacOSのプライバシー設定を開けます。
4. 検出結果を確認して手動で隔離します。ファイルのハッシュ確認と移動後の再検査を行い、復元時は既存ファイルを上書きしません。隔離は同じファイルシステム内での原子的な移動に限定します。外付けボリュームからの隔離はエラーとして安全に中断します。
5. 履歴はSQLiteに保存し、JSONでエクスポートできます。操作ログは左右にドッキングするか、独立したウインドウにできます。

アプリ起動中のみ予約検査と6時間ごとの定義更新を実行します。アプリ終了中に常駐するサービスやリアルタイム遮断はありません。アクセス不能・サイズ制限は検査未完了として表示します。XProtectとGatekeeperを引き続き利用してください。

## 検証

```sh
npm run build
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```

ClamAVがある環境では、独自の無害なテスト署名を使った実プロセス検証も可能です：

```sh
cargo test --manifest-path src-tauri/Cargo.toml actual_clamav_streams_clean_and_detected_files -- --ignored
```

公式PKG検証テストは、公式のPKGと同名の`.sig`を用意して`CLAMAV_TEST_PKG`にPKGのパスを指定します。Apple署名確認には通常のmacOS信頼サービスへのアクセスが必要です。このテストはインストールしません。

実装と検証の進捗は[roadmap](docs/roadmap.md)、構成は[architecture](docs/architecture.md)を参照してください。

## English

A macOS ClamAV frontend with Rust-owned state and a Tauri 2 WebView. It supports engine discovery, verified official installer downloads, FreshClam updates, cancellable scans, SQLite history, explicit quarantine and safe restore, daily scheduling while the app is open, and a dockable/floating activity panel. No files are automatically deleted. External-volume quarantine is intentionally rejected. The app itself is an unsigned development build; native installer authentication remains in macOS Installer. Run `npm ci` then `npm run tauri dev`.

## 简体中文

这是基于Tauri 2与Rust的macOS ClamAV前端，支持引擎检测、官方安装包签名验证、FreshClam更新、可停止的扫描、SQLite历史记录、手动隔离与安全恢复，以及可停靠或浮动的操作日志。支持英语、日语和简体中文。预约扫描仅在应用运行期间执行，不提供实时拦截，不自动删除文件。跨卷隔离会安全中止。当前应用是尚未签名及公证的开发版本；安装引擎时请在macOS Installer中完成管理员认证。执行`npm ci`后运行`npm run tauri dev`。
