# ワーカーの出力回収が入力処理を starvation させないようにする

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-worker-starvation
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Encoder` / `Decoder` のワーカーは、`recv_timeout(1ms)` のタイムアウト時にだけ `drain_output` を呼ぶ。`SubmitInput` が連続するとコマンド処理が優先され、出力回収が滞る。その結果、AMF の入力キューが詰まって `AMF_INPUT_FULL` が返り、100 回 (約 100 ms) のリトライ上限に達してエラーになる可能性がある。

## 現状

- `src/encode.rs` / `src/decode.rs` のワーカーは、`recv_timeout(1ms)` でコマンドを受け取り、タイムアウトしたときだけ `drain_output` を呼ぶ。
- `SubmitInput` のリトライは最大 100 回で、`AMF_INPUT_FULL` が続くとエラーになる。
- 連続してフレームを投入するストレステストがない。
- `finish` の待機中も同じポーリング経路を通る。

## 設計方針

- コマンドを 1 つ処理した後にも `drain_output` を呼び、入力と出力の両方を進める。
- コマンドキューに複数の `SubmitInput` が溜まっている場合でも、一定回数ごと、または各コマンドの後に出力回収を挟む。
- `recv_timeout` の待機時間と `drain_output` の呼び出し頻度を、入力と出力のバランスが取れるように調整する。
- `AMF_INPUT_FULL` のリトライ中にも `QueryOutput` を呼び、出力を回収して入力キューを空ける。
- 連続投入のストレステストを追加し、`AMF_INPUT_FULL` のリトライ上限に達しないこと、全フレームが出力されることを検証する。
- 出力の到着が遅いエンコーダーでも、入力の受付が過度に遅れないことを確認する。

## 完了条件

- 連続してフレームを投入しても、`AMF_INPUT_FULL` のリトライ上限で失敗しない。
- 入力と出力の両方が進み、全フレームが出力される。
- ストレステストが追加され、`cargo test --workspace` が通る。
- ワーカーのポーリング方針がコメントで説明されている。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。
