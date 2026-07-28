# SIT TOTP for Windows

芝浦工業大学のMicrosoft Authenticator登録から取得したTOTPシードを使い、現在の6桁コードを生成する軽量なWindows常駐アプリです。

ブラウザ拡張機能 [`SIT-TOTP-AutoFill`](https://github.com/atuy1219/SIT-TOTP-AutoFill) と同じ、次の方式に固定しています。

- HMAC: SHA-256
- 桁数: 6桁
- 更新間隔: 30秒

WebView、Electron、常駐Webサーバーは使用せず、RustとWin32 APIだけでUI・タスクトレイ・クリップボード・自動起動を実装しています。

## 機能

- タスクトレイに常駐
- 現在のTOTPコードと残り秒数を表示
- ボタン、トレイメニュー、`Ctrl + Alt + T`でコードをコピー
- Base32シードと`otpauth://` URIに対応
- シードをWindows DPAPIで現在のWindowsユーザーに紐づけて暗号化保存
- Windowsログオン時の自動起動
- 二重起動防止
- 最小化・閉じる操作では終了せずトレイへ格納

## 使い方

1. ReleasesまたはGitHub Actionsのartifactから`SIT-TOTP-For-Windows.exe`を取得します。
2. 起動後、拡張機能と同じTOTPシードを入力して「保存」を押します。
3. Microsoft Authenticatorに表示されるコードと一致することを確認します。
4. 必要に応じて「Windowsログオン時に自動起動する」を有効にします。

シードの取得方法は[`SIT-TOTP-AutoFill`のREADME](https://github.com/atuy1219/SIT-TOTP-AutoFill#4-totpシードを取り出す)を参照してください。

> [!CAUTION]
> TOTPシードは認証コードを生成できる秘密情報です。Issue、ログ、スクリーンショット、ソースコードへ記載しないでください。

## シードの保存

シードは以下へ保存します。

```text
%LOCALAPPDATA%\SIT-TOTP\seed.dat
```

内容はWindows DPAPIのユーザー単位鍵で暗号化されます。原則として、保存したWindowsユーザー以外では復号できません。アプリを別PCへ移す場合は、元のBase32シードを再入力してください。

## ビルド

Windows上でRustをインストールし、リポジトリのルートで実行します。

```powershell
cargo build --release
```

生成物:

```text
target\release\sit-totp-for-windows.exe
```

テスト:

```powershell
cargo test
```

## 配布サイズ・常駐負荷

ネイティブWin32アプリのため、ElectronやWebViewベースのアプリより実行ファイルと常駐メモリを抑えられます。ReleaseビルドではLTO、サイズ最適化、シンボル削除を有効にしています。

## 注意事項

このアプリは個人の研究・利便性向上を目的とした非公式ツールであり、芝浦工業大学およびMicrosoftとは関係ありません。大学側またはMicrosoft側の認証方式変更により利用できなくなる可能性があります。
