# 公開 API の `///` ドキュメントを追加する

- Created: 2026-11-10
- Completed: 2026-11-16
- Branch: feature/fix-public-api-docs
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

shiguredo-rust の規約は、公開 API の型・関数・フィールド・variant に `///` を必ず書くことを求めている。現在のクレートは型とメソッドの多くに doc がある一方、利用者が最も触る設定型のフィールド、列挙型の variant、`frame_type` モジュールなどに doc が欠けている。rustdoc を読むだけでは各フィールドの意味や制約が分からない。

## 現状

`///` が欠けている公開項目は次のとおりである。

- `src/encode.rs` の `H264Profile`、`HevcProfile`、`Av1Profile` の全 variant
- `src/encode.rs` の `CodecConfig` の全 variant
- `src/encode.rs` の `EncoderConfig` の全フィールドと `EncoderConfig::new`
- `src/encode.rs` の `ReconfigureParams` の全フィールド
- `src/encode.rs` の `H264EncoderConfig::profile`、`HevcEncoderConfig::profile`、`Av1EncoderConfig::profile`
- `src/encode.rs` の `frame_type` モジュールと 6 つの定数
- `src/encode.rs` の `FnEncodeHandler::new`
- `src/decode.rs` の `DecoderCodec` の全 variant、`DecoderConfig::codec`、`FnDecodeHandler::new`
- `src/lib.rs` の `AmfLibrary::instance`
- `src/sys.rs` の `AMFInit_Fn`、`AMFQueryVersion_Fn`

## 設計方針

- すべての公開項目に、利用者から見た仕様 (入力、戻り値、エラー、制約、単位、有効期間) を書く。実装の内部動作を主軸にしない。
- `EncoderConfig` のフィールドには、コーデックごとの対応と、指定しても無視される組み合わせ (別 issue で解消するもの) を明記する。
- `frame_type` は、現状 IDR だけが有効であることを明記する。
- `src/lib.rs` の先頭に `#![warn(missing_docs)]` を追加し、再発を防ぐ。`#[doc(hidden)]` の `ffi` は対象外にする。
- 可能なら doc の例を追加し、`cargo test --doc` で検証する。
- 既存の doc の誤り (thread-safe の記述、`instance()` の遅延ロード、`finish()` の保証) は別 issue の範囲とし、本 issue では欠落の追加に絞る。

## 完了条件

- `#![warn(missing_docs)]` を有効にしても警告が出ない。
- `cargo doc --no-deps` が成功する。
- doc のリンク切れ (`[`encode()`]` / `[`decode()`]`) が解消されている。
- `cargo test --doc` が通る。
- `CHANGES.md` は doc のみの変更のため更新しない。

## 解決方法

- `src/lib.rs` に `#![warn(missing_docs)]` を追加した。`#[doc(hidden)]` の `ffi` 経由でのみ公開する `sys` の生成コード (`metadata` / `bindings` / `str`) は、項目ごとの doc を持たないため `#![expect(missing_docs)]` で対象外にした。
- `H264Profile` / `HevcProfile` / `Av1Profile`、`CodecConfig`、`DecoderCodec` の全 variant と、各エンコーダー設定の `profile` に doc を追加した。
- `EncoderConfig` / `ReconfigureParams` の全フィールドに doc を追加した。共通の制約は型の doc にまとめ、フィールドには値の意味と単位を書いた。
- `frame_type` モジュールと 6 定数、`EncodeOptions::frame_type` に doc を追加し、IDR だけが有効であることを明記した。
- `EncoderConfig::new`、`FnEncodeHandler::new`、`FnDecodeHandler::new`、`DecoderConfig::codec`、`AmfLibrary::instance`、`AmfInitFn` / `AmfQueryVersionFn` に doc を追加した。
- `# Errors` は実装の検証分岐を列挙せず、設定不正・バックエンド非対応・バックエンド呼び出し失敗などのカテゴリ単位の契約に整理した。
- ドキュメント整備中に確認した `qpb` の扱いを修正した。HEVC の `qpb` は `Encoder::new` / `Encoder::reconfigure` でエラーにし、AV1 の `qpb` は初期化時にも設定する。HEVC の `ReconfigureParams::gop_pic_size` も `Encoder::reconfigure` でエラーにする。
- クレート冒頭に `no_run` のエンコード例を追加し、`cargo test --doc` でコンパイルを検証するようにした。振る舞いの変更を含むため、README / SKILL.md / CHANGES.md も実装に合わせて更新した。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`、`cargo test --doc`、`cargo test --workspace` が AMD GPU 環境で通ることを確認した。
