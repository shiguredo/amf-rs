# 公開 API の `///` ドキュメントを追加する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
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
