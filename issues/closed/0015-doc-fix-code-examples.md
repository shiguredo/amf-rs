# README / SKILL.md のコード例をコンパイル可能な形に修正する

- Created: 2026-11-10
- Completed: 2026-11-16
- Branch: develop
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

## 解決方法

- ドキュメント修正のため `develop` ブランチで直接対応した
- `README.md` と `skills/shiguredo-amf/SKILL.md` のエンコード例 / デコード例を `FnEncodeHandler::new` / `FnDecodeHandler::new` でクロージャを包む形に修正した
- クロージャの引数を `Result<EncodedFrame<T>, E>` / `Result<DecodedFrame<T>, E>` に修正し、ハンドラーへ通知されるエラーを処理する例にした
- 結果の受け取りを `Arc<Mutex<Vec<_>>>` から `mpsc::channel` に変更し、`finish()` 後に `try_iter()` で回収する例にした
- `SKILL.md` の `AmfLibrary::create_context()` を `AmfLibrary::instance().create_context()` に修正した
- `SKILL.md` の `AmfLibrary::instance()` が `dlopen` を行うという記述を、初回の `query_version()` / `create_context()` / `create_component()` 呼び出しで行うという記述に修正した
- `SKILL.md` の `finish()` の保証に関する記述は、タイムアウトが廃止されて現在の実装と一致していたため変更していない
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`prek run --all-files` が通ることを確認した
- README のコード例を doctest または `examples/` で検証する対応は行っていない。`cargo test --doc` の対象に README は含まれておらず、README の例はコンパイル検証されていない
- AMD GPU が無い環境のため `cargo test --workspace` の実機テストは実行していない
