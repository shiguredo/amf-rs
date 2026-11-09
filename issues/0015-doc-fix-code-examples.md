# README / SKILL.md のコード例をコンパイル可能な形に修正する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-code-examples
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

README と `skills/shiguredo-amf/SKILL.md` のコード例が実際にはコンパイルできない。`Encoder::new` / `Decoder::new` にクロージャを直接渡しているが、`EncodeHandler` / `DecodeHandler` を実装するのは `FnEncodeHandler` / `FnDecodeHandler` だけで、クロージャ用の blanket impl は存在しない。実機で試すと `E0277: the trait bound closure: EncodeHandler is not satisfied` になる。

また、クロージャの引数型は `EncodedFrame<T>` / `DecodedFrame<T>` になっているが、実際の通知は `Result<EncodedFrame<T>, E>` / `Result<DecodedFrame<T>, E>` である。`FnEncodeHandler` で包んでも二重に型エラーになる。

## 現状

- `README.md` のエンコード例とデコード例がクロージャを直接渡している。
- `skills/shiguredo-amf/SKILL.md` の例も同じ書き方で、さらに「`FnMut(Result<EncodedFrame<T>, E>) + Send + 'static` であれば暗黙的に `FnEncodeHandler` 相当として扱える」と書いているが、事実と異なる。
- `SKILL.md` の「`instance()` が初回呼び出し時に `libamfrt64.so.1` を `dlopen` する」という記述も誤りである。`dlopen` は `AmfLibrary::ensure_inner` (最初の `query_version` / `create_context` / `create_component`) で起きる。
- `SKILL.md` の「`finish()` は残りのフレームをハンドラーが受け取り終わるまで待機する」という記述は、5 秒でタイムアウトして `Err` を返し、未処理の出力を破棄する実装と一致しない。
- README の例は doctest や `examples/` で検証されていない。

## 設計方針

- 例を `FnEncodeHandler::new(...)` / `FnDecodeHandler::new(...)` に修正し、クロージャの引数を `Result<EncodedFrame<T>, E>` / `Result<DecodedFrame<T>, E>` にする。
- README のエンコード例とデコード例を `src/lib.rs` の doctest (`no_run`) または `examples/` に移し、`cargo test --doc` で検証できるようにする。
- `SKILL.md` のクロージャ、`instance()` の遅延ロード、`finish()` の保証に関する誤記を修正する。
- thread-safe の記述 (「すべて `unsafe impl Send` 済み」) は別 issue の範囲なので、本 issue では扱わない。
- 修正後の例が実際にコンパイルできることを CI で確認する。

## 完了条件

- README と `SKILL.md` のコード例がコンパイルできる形になっている。
- 例が doctest または `examples/` で検証される。
- `SKILL.md` の誤記 3 件 (クロージャ、`instance()`、`finish()`) が修正されている。
- `cargo test --doc` と `cargo test --workspace` が通る。
