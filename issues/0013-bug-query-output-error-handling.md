# `QueryOutput` のエラーと `finish` 時の未マッチ出力を通知する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-query-output-error-handling
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`QueryOutput` が `AMF_OK` / `AMF_REPEAT` / `AMF_EOF` 以外の結果を返した場合、現在は理由を通知せずにループを抜ける。`pending` が減らないため、ワーカーは 1ms 間隔のポーリングを続け、`finish()` が 5 秒後に `Finish wait timed out` を返す。ハンドラーには何のエラーも届かない。

また、ワーカーの終了条件は `pending` が空になったことだけを見ており、`output_buffer` に残った未マッチの出力は無通知で破棄される。出力数が入力を上回る場合 (デコーダーの並べ替えや 1 入力複数出力など)、利用者は欠落に気付けない。

## 現状

- `src/encode.rs` の `drain_output` と `src/decode.rs` の `drain_output` は、`AMF_REPEAT` / `AMF_EOF` 以外の非 OK を `break` で捨てる。
- `output_buffer` は `Result<T, Error>` を保持できるが、`QueryOutput` 自体のエラーは積まれない。
- ワーカーの終了条件は `finish.is_some() && pending.is_empty()` であり、`output_buffer` の残量を見ていない。
- `finish()` のタイムアウトは 5 秒固定で、残りの `pending` 数や最後の `QueryOutput` の結果をエラーメッセージに含めていない。

## 設計方針

- `QueryOutput` の非 OK を `Err` として `output_buffer` に積み、対応する `pending` を 1 つ消費してハンドラーへ通知する。
- `AMF_OK` かつ出力が null の場合は「出力なし」として継続する。AMF の仕様上あり得るため、エラーと区別する。
- ワーカーの終了前に必ず `drain_output` を実行し、`pending` と `output_buffer` の両方が空になったことを確認する。残った場合はエラーとして返す。
- `finish()` のタイムアウト時は、残りの `pending` 数と最後の `QueryOutput` の結果をエラーメッセージに含める。
- マッチング処理を純粋関数に切り出し、合成したキューで単体テストする。AMF を模したモックは使わない。
- エンコーダーとデコーダーの両方に同じ方針を適用する。

## 完了条件

- `QueryOutput` の非 OK がハンドラーに `Err` として通知される。
- `finish()` が `output_buffer` の残りを無通知で破棄しない。
- タイムアウト時のエラーから残りの `pending` 数と最後の結果が分かる。
- マッチング処理の単体テストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。
