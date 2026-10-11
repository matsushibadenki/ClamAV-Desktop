# macOS release

## 開発・検証用DMG

```sh
npm ci
npm run dist:mac
```

Rustとフロントエンドのテスト後、releaseモードの`.app`とDMGを作成します。開発用はad-hoc署名を明示し、個人のApple認証情報をビルド環境から除外します。署名の整合性、DMGの整合性を検証し、`src-tauri/target/release/bundle/verification/`にSHA256SUMSとrelease.jsonを作成します。**開発用DMGはDeveloper ID署名・公証済みの配布物ではありません。**

## 正式な署名・公証

1. キーチェーンに有効なDeveloper ID Application証明書を用意します。
2. `APPLE_SIGNING_IDENTITY`にその証明書名を設定します。
3. 公証用に、`APPLE_ID`・`APPLE_PASSWORD`（アプリ用パスワード）・`APPLE_TEAM_ID`、または`APPLE_API_ISSUER`・`APPLE_API_KEY`・`APPLE_API_KEY_PATH`を実行環境で設定します。値や秘密鍵はリポジトリに保存しません。
4. `npm run release:mac -- --check`で事前確認します。
5. Appleへの公証送信を行う正式なリリース時に`npm run release:mac`を実行します。Tauriが署名・公証し、スクリプトがcodesign・Gatekeeper・staplerでアプリを検証します。アプリの公証済み状態と、DMGのコンテナ整合性は別に記録します。

環境変数と署名方式は[Tauriの公式ガイド](https://v2.tauri.app/distribute/sign/macos/)に従っています。スクリプトは秘密値を出力せず、証明書や公証用情報が不足するとビルド前に停止します。

## 更新手順

- package.json、src-tauri/Cargo.toml、src-tauri/tauri.conf.jsonのバージョンを揃え、npmのロックファイルとCargo.lockも更新します。事前確認で不一致を拒否します。
- 同梱するClamAVの公式バージョンを変更する場合は、installer.rsとInstaller.tsxの表示を更新し、公式鍵・PGP・Apple署名・Gatekeeper検証テストを再実行します。
- 対象CPUごとのビルドを作成し、同じCPUのクリーンなMacでDMGから起動・エンジン導入・検出・隔離・復元・再起動を確認します。通常のローカルビルドは実行したMacのCPU用です。Intel・Universalは別途検証が必要です。
- 配布するDMGに検証ファイルのSHA-256が一致することを確認します。スクリプトは公開やアップロードを行いません。アプリの自動アップデーターは未実装です。

## English

`npm run dist:mac` produces an ad-hoc development app and DMG, runs tests, verifies their integrity, and writes checksums. It does not claim notarization. For distribution, configure a valid Developer ID Application identity and notarization credentials in the environment, run `npm run release:mac -- --check`, then deliberately run `npm run release:mac` when ready to submit the app to Apple. Keep all three version declarations synchronized. The script never publishes releases.

## 简体中文

`npm run dist:mac`生成ad-hoc签名的开发应用与DMG，执行测试、验证完整性并生成SHA-256文件。开发构建未经过正式签名及公证。正式发布需在环境中设置有效的Developer ID Application证书和公证凭据，先运行`npm run release:mac -- --check`，准备向Apple提交公证时再运行`npm run release:mac`。请同步三个版本声明。脚本不会发布或上传发行版。
