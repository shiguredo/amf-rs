# 未使用の AMF ラッパー API を削除する

- Created: 2026-11-10
- Completed: 2026-11-18
- Branch: feature/refactor-remove-unused-api
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`src/amf.rs` などの AMF ラッパーに、クレート内外から未使用の API が残っている。公開 API の表面積を減らし、誤用 (特に参照カウントを増やさない生ポインタの受け渡し) を防ぐ。

## 現状

未使用の API は次のとおりである。

- `PropertyStorage` / `Surface` / `Buffer` / `Plane` / `Context` / `Component` の `into_raw` (6 種)。
- `PropertyStorage` / `Plane` / `Component` の `as_ptr` (3 種)。
- `set_duration` / `get_duration` / `get_vpitch`。
- `from_raw` / `from_raw_acquired` の未使用側。内部で使うのは `PropertyStorage::from_raw_acquired` と `Plane::from_raw_acquired` だけである。
- `AMFVariantStruct::from_bool` / `from_double` / `from_ratio`。対応する `AMF_VARIANT_*` を使う箇所がない。
- `Component::get_context`。参照カウントを増やさない生ポインタを返す。
- `src/encode.rs` の `buf_native` の null チェック。値を一切使わないため無意味である。

`Component::re_init` は解像度変化の issue で使う予定があり、`Decoder::flush` は `finish` の issue で扱うため、削除対象から外す。

## 設計方針

- 上記の未使用 API を削除する。公開 API の削除は破壊的変更になるため、`CHANGES.md` に `[CHANGE]` として記載する。
- 削除前に、クレート内のテストと example から参照がないことを確認する。
- 安全なフレーム読み書き API の issue で追加する API と重複しないことを確認する。
- `get_context` のような参照カウントを増やさない API を残す場合は、`unsafe` と Safety 注記を付ける。今回は削除を基本とする。
- `buf_native` の null チェックを削除し、`buf_size == 0` のチェックだけを残す。
- 削除後に `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通ることを確認する。

## 完了条件

- 未使用の `into_raw` / `as_ptr` / `set_duration` / `get_duration` / `get_vpitch` / 未使用の `from_raw` / `AMFVariantStruct` の未使用コンストラクタ / `get_context` が削除されている。
- `buf_native` の無意味な null チェックが削除されている。
- `re_init` と `flush` は残っている。
- `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記する。

## 解決方法

対応しない。公開クレートの API を「クレート内で未使用」という理由で削除する方針は採用しない。`shiguredo_amf` は crates.io で公開されており、外部からの利用の有無をリポジトリ内の参照だけで判断できない。現在のテストも `Surface` / `Buffer` / `Plane` / `PropertyStorage` の `into_raw` / `from_raw`、`set_duration`、`get_duration`、`get_vpitch`、`Context::as_ptr` を使用しており、「クレート内外から未使用」という前提は成立していない。

`Component::get_context` は AMF の `GetContext()` が返す借用ポインタを扱う API であり、参照カウントを持つ `Context` を返す `Component::context()` とは所有権と寿命の契約が異なる。重複として削除できるものではない。`as_ptr` / `into_raw` などの低レベル API は、`get_native` などと同様に interop 用の escape hatch として残す。誤用を防ぐために削除するのではなく、返されるポインタの有効期間と `Release` の責任を doc に明記する。

`buf_native` の null チェックは公開 API とは無関係な内部実装の話であり、本 issue の範囲から外す。`AMFVariantStruct` の未使用コンストラクタは `ffi` 側の semver 対象外ヘルパーであり、本 issue では扱わない。
