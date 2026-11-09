# shiguredo-rust / AGENTS.md の規約違反 (`#[allow]`、`.unwrap()`、`log`、英語コメント) を解消する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-coding-conventions
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

shiguredo-rust と AGENTS.md の規約に違反している箇所を解消する。機能の変更は行わない。

- `#[allow(...)]` を 6 箇所で使用している。規約は例外なしで禁止し、`#[expect(...)]` を使うことを求めている。`src/sys.rs` の 5 つは file-level で、同じファイルの手書きコード (`AMFVariantStruct` のヘルパー、`to_wstring`) にも適用されている。`src/encode.rs` の `too_many_arguments` は関数を分割すれば不要になる。
- 本体コードで `.unwrap()` を 12 箇所、テストで 16 箇所使用している。規約は `.expect("MESSAGE")` を使うこと、本体コードのメッセージは英語、テストのメッセージは日本語とすることを求めている。`src/lib.rs` の `lock().unwrap()` は Mutex poisoned 時にパニックする経路で、メッセージがない。
- ログに `log` クレートを使用している。規約は `tracing` と `tracing-subscriber` を使うこと。
- `src/encode.rs` に英語の実装コメント (`Profile`、`Frame size`、`Rate control`、`Usage`) があり、`tests/test_roundtrip.rs` にも `BT.601 RGB -> YCbCr` がある。AGENTS.md はコメントをすべて日本語にすることを求めている。
- `FrameFormat::frame_size` が `&self` を取る。Copy な enum のメソッドは `self` で値を受け取る規約に反する。
- `src/lib.rs` の `Mutex` に、ロックが必要な理由のコメントがない。

## 現状

- `src/sys.rs` の `#![allow(non_camel_case_types)]`、`#![allow(non_snake_case)]`、`#![allow(non_upper_case_globals)]`、`#![allow(dead_code)]`、`#![allow(clippy::all)]`。
- `src/encode.rs` の `#[allow(clippy::too_many_arguments)]`。
- `src/lib.rs`、`src/encode.rs`、`src/decode.rs` の `.unwrap()`。
- `Cargo.toml` の `log = "0.4"` と、`src/encode.rs` / `src/decode.rs` の `log::debug!`。
- `src/encode.rs` と `tests/test_roundtrip.rs` の英語コメント。
- `src/encode.rs` の `FrameFormat::frame_size(&self, ...)`。
- `src/lib.rs` の `Mutex<Option<AmfLibraryInner>>`。

## 設計方針

- `#[allow]` を `#[expect]` に置き換える。bindgen の生成コードは `include!` する専用モジュールに閉じ、手書きコードに lint の抑制が及ばないようにする。生成コードで発火しない `#[expect]` は削除する。`too_many_arguments` はプロパティ名を構造体にまとめるなどして関数を分割する。
- `.unwrap()` を `.expect("MESSAGE")` に置き換える。本体コードは英語、テストは日本語のメッセージにする。Mutex は `poisoned` を明示するメッセージにする。
- `log` を `tracing` に置き換える。ホットパスで毎回出している `debug!` は削除するか `trace!` に落とす。`Cargo.toml` の依存には用途コメントを付ける。
- 英語の実装コメントを日本語にする。
- `FrameFormat::frame_size` を `self` で受け取るようにする。
- `Mutex` を使う理由 (AMF ランタイムのグローバル状態を直列化するため) をコメントに書く。
- 機能の変更は行わない。挙動が変わらないことを既存テストで確認する。

## 完了条件

- `src/` と `tests/` に `#[allow(...)]` と `.unwrap()` が残っていない。
- `log` 依存がなくなり、ログは `tracing` に統一されている。
- コメントがすべて日本語になり、他モジュールへの言及、未来の状態の説明、実態と不一致の記述が修正されている。
- `FrameFormat::frame_size` が `self` を受け取る。
- `Mutex` の理由コメントがある。
- `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` の `### misc` にリファクタリングとして追記する。
