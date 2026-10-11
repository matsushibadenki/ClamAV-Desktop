# Roadmap

- 🟢 [Done] Tauri 2 + RustのDocument/State/Renderer構成と5画面のUI。
- 🟢 [Done] 英語・日本語・简体中文、狭い画面に対応したレイアウト。
- 🟢 [Done] 公式PKG/Homebrewエンジン検出と既存定義の読み取り。
- 🟢 [Done] 公式PKGダウンロード、固定Cisco鍵によるPGP検証、Apple署名・公証検証、macOS Installerへの受け渡し。
- 🟢 [Done] FreshClamによるアプリ専用定義更新と起動中の定期更新。
- 🟢 [Done] クイック・フォルダ・ホーム検査、停止、実件数表示、未検査ファイルの通知。
- 🟢 [Done] SQLite履歴、JSON出力、再起動後の中断検査復旧。
- 🟢 [Done] ハッシュ照合、再検査、隔離・上書きしない復元、途中操作のジャーナル復旧。
- 🟢 [Done] 起動中の毎日予約検査、Rustが共有状態を管理するログパネルの左右ドッキング・独立ウインドウ。
- 🟠 [Next] Developer IDによるアプリ署名・公証と配布用DMG、リリース更新手順。
- 🟠 [Next] 外付けボリューム隔離をコピー・同期・検証・復旧まで保証する設計で追加。
- 🔴 [Later] clamd常駐サービス、ログパネル以外のレイアウト拡張、古いダウンロードの整理。
- 🔴 [Later] Endpoint Security Entitlementを取得した場合のリアルタイム検査。
- ⭕️ [Pending] 未導入のクリーンなMacでの管理者インストール完了、Intel Mac、macOS 12での実機検証。
- ⭕️ [Pending] アプリ終了中の予約実行には常駐サービスの設計・実装・検証が必要。
