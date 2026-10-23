# Hengband score server — Rust migration

既存のスコア送信形式を受け付け、SQLiteへの登録とWeb表示を行うRust実装。
Gitはソースとビルド入力の管理に使用し、DB・ダンプ・画面・資格情報・配備先固有の設定を保存しない。

## ビルドと検証

Linux、Rust（`rust-toolchain.toml` に固定）、システムのSQLite開発ライブラリとpkg-configが必要。
Cargo.lockも追跡し、依存更新は明示的に行う。

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked --bin hengscore-server
```

通常のビルド・RustテストにPHP実行環境や実データは不要。
Gitに含まれるRustテストと、配備環境で実施したDB・HTTP・常駐化・復元の外部検証は別の範囲。
実行バイナリにはシステムSQLiteへの動的リンクがあるため、配備先でもSQLiteランタイムが必要。

GitHub Actionsはpush・pull request・手動実行で、fmt・Clippy・Rustテスト・cargo auditを独立して実行する。
DBやダンプ、VPSへの接続、追加のGitHub Secretsは不要。checkoutにはGitHubが発行する読み取り専用のトークンを使い、認証情報をcheckout後に保持しない。
checkout actionはコミットSHA、cargo-auditはバージョンを固定する。依存監査は最新の公開アドバイザリを取得し、警告も失敗として扱う。
このCIはコード検証のみを行い、デプロイや公開URLの変更は行わない。

## 実行

```sh
hengscore-server --store /absolute/path/to/store --store-id STORE_ID --port 18090
```

待受はコードで127.0.0.1に固定。外部公開やTLS終端はこの実装の外で構成する。
ストアは別途準備し、`db/score.db`、`dumps/`、`screens/`、`.staging/` が必要。
`.hengscore-store` は次の形式で、`STORE_ID` は32桁の小文字16進数。起動時に識別子を照合し、未完了の登録を回復してからREADYを通知する。

```text
hengscore-store-v1
STORE_ID
```

このREADMEの形式例だけではDBスキーマ・参照データ・配備構成は作成されない。空のDBを自動生成して本番サービスとして公開しない。
`src/bin/*_probe.rs` 等は開発用。常駐用の実行ファイルは `hengscore-server`。
旧形式の受付キーがない投稿は、応答途絶後の再送が別投稿として登録される可能性を維持している。

## 文字コード互換と出典

`src/legacy_encoding.rs` とJIS表は旧SJIS／EUC-JP変換との互換処理を行う。
`php-reference/` にはテスト用期待値、その生成に用いた固定参照ソース、出典・ライセンスを保存する。
Rustテストは `php-reference/compat_vectors.bin` をコンパイル時に読み込むため、このファイルも必須。
第三者由来の表・参照コードについては `php-reference/NOTICE` と `php-reference/LICENSE` を参照。
英語の名称対応表の出典とハッシュは `english_names_source.json` に保存している。

## 管理範囲

`.gitignore` は許可したソース・ビルド入力だけを追跡できる構成。運用スクリプト、設定草案、検証結果、画像、秘密鍵、DB、ダンプ、画面、targetは追跡しない。
Gitの履歴はデータバックアップを代替しない。データ保全は運用側で別管理する。
配備先のバイナリや稼働サービスは、コミットしただけでは更新されない。

## ライセンス

このリポジトリのRustのコードとビルド入力はMITライセンス（`LICENSE`）。

ただし次のファイルはPHP libmbfl（"streamable kanji code filter and converter"、Copyright (c) 1998-2002 HappySize, Inc.）に由来し、
GNU Lesser General Public License version 2.1（`php-reference/LICENSE`）の下で再配布する。MITの対象外。

- `php-reference/` 配下の参照ソースと、そこから生成した `compat_vectors.bin`・`decoder_oracle.c`
- `src/jis0208.bin`・`src/jis0212.bin`（`unicode_table_jis.h` の表をそのまま16ビットに詰めたもの）

JIS表は `hengscore-server` の実行バイナリに組み込まれるため、バイナリを配布する場合はLGPLの条件も満たすこと（このリポジトリのソース公開で満たせる）。
出典のコミットとSHA-256は `php-reference/NOTICE` と `php-reference/extraction_report.json` を参照。
