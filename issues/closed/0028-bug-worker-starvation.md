# ワーカーの出力回収が入力処理を starvation させないようにする

- Created: 2026-11-10
- Completed: 2026-11-20
- Branch: feature/fix-worker-starvation
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Encoder` / `Decoder` にフレームを連続して投入するとき、`SubmitInput` のリトライ中に出力を回収できないと入力キューや出力用 Surface が空かず、リトライ上限に達して操作が失敗する。入力の処理と出力の回収を並行して進め、連続投入しても全フレームが出力されることを保証する。

## 現状

- `src/worker.rs` の `submit_input_with_retry` は、呼び出し元スレッドで `SubmitInput` を 1 ms 間隔で最大 100 回リトライする。対象は `AMF_INPUT_FULL` と `AMF_DECODER_NO_FREE_SURFACES` である。
- 出力の回収はワーカースレッドが行う。ワーカーは `RegisterUserData` を 1 つ処理するたびに `drain_output` を呼び、保留中の入力または出力がある間は 1 ms 間隔で `QueryOutput` を呼ぶ。
- `WaitEof` 中の `Drain` が `AMF_INPUT_FULL` を返した場合も、ワーカーは出力を回収しながら再試行し、`AMF_EOF` まで回収する。
- `AMF_INPUT_FULL` を返す状況では受理済みで出力が未回収の入力があるため、ワーカーの保留一覧は空にならない。ワーカーはリトライ中も `QueryOutput` を呼び続けられる。
- エンコーダーとデコーダーに、200 フレームを待たずに投入するストレステストがある。

## 設計方針

- `SubmitInput` は呼び出し元スレッド、`QueryOutput` はワーカースレッドで行い、入出力を並行して進める。
- ワーカーはコマンドを処理するたびに出力を回収し、コマンド待ちの間も保留中の入出力がある限り定期的に出力を回収する。
- `AMF_INPUT_FULL` と `AMF_DECODER_NO_FREE_SURFACES` のリトライ中も、ワーカーの出力回収によって入力キューまたは出力用 Surface を空ける。
- `finish` は `Drain` の `AMF_INPUT_FULL` を出力回収しながら再試行し、`AMF_EOF` まで待つ。
- ストレステストで、エンコーダーとデコーダーの両方が全フレームを出力し、`user_data` の欠落や重複がないことを確認する。

## 完了条件

- 連続してフレームを投入しても、`SubmitInput` のリトライ上限に達してエラーにならない。
- 入力と出力の両方が進み、全フレームが出力される。
- 連続投入のストレステストが追加され、`cargo test --workspace` が通る。
- ワーカーのポーリング方針がコメントで説明されている。

## 解決方法

`src/worker.rs` の共通ワーカーがコマンド処理後と保留中の入出力がある間に `QueryOutput` を呼ぶ構造になっていたため、starvation を防ぐための追加の実装変更は不要だった。

回帰防止として、200 フレームを待たずに投入するストレステストを追加した。`tests/test_encode.rs` の `test_encoder_continuous_submit_outputs_all_frames` は、すべての `encode` が成功すること、全フレームが出力されること、`user_data` に欠落や重複がないことを確認する。`tests/test_decode.rs` の `test_decoder_continuous_submit_outputs_all_frames` も同様に `decode` を確認する。

AMD GPU の実機で `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all --check` が成功している。既定の入力キューではエンコーダーの `AMF_INPUT_FULL` を再現できなかったため、リトライ中の出力回収は、`AMF_DECODER_NO_FREE_SURFACES` のリトライが発生するデコーダーのストレステストで確認した。

## 追加調査

既定の入力キューではエンコーダーの `AMF_INPUT_FULL` が発生しなかったため、AMF の仕様と入力キューの深さを変えた条件で原因を調べた。

- AMF の API Reference では、`SubmitInput` は出力が適切に drain されていないときに `AMF_INPUT_FULL` を返す。入力キューの既定値は 16 で、設定範囲は 1-32 である。
- 現行のワーカーはコマンド処理後と保留中の入出力がある間に出力を回収するため、既定のキュー 16 を埋める前に出力が回収される。
- 320x240 の H.264 で 200 フレームを連続投入した場合、`SubmitInput` は 1 フレームあたり約 0.6 ms かかり、キューは 16 まで溜まらない。
- 入力キューの深さを 1 に強制すると、現行コードで `AMF_INPUT_FULL` が 599 回発生した。リトライ中もワーカーが出力を回収し、200 フレームすべてが出力された。
- 0032 以前のコードでも同じ条件で 596 回発生し、200 フレームすべてが出力された。リトライ中は 1 ms の sleep が入り、その間にワーカーが drain するため、100 回のリトライ上限には達しなかった。
- B フレームを 3 にした 320x240 の 200 フレーム投入では、0032 以前のコードが `AMF_INPUT_FULL` を 9-10 回発生させたのに対し、現行コードは 0 回だった。コマンド処理後の drain は `AMF_INPUT_FULL` の発生回数を減らす効果がある。
- 0032 以前のコードで 1080p にすると、既定のキューでは `AMF_INPUT_FULL` は発生しなかった。解像度を上げるだけでは再現条件にならない。

入力キューには `InputQueueSize` で上限を設定できるが、amf-rs はこの設定を公開していない。追加調査では、隔離した検証環境で一時的に `InputQueueSize=1` を設定した。H.264 の `AMF_VIDEO_ENCODER_INPUT_QUEUE_SIZE` 定数は、`build.rs` のプロパティ抽出が `#define` の二重スペースを扱えず `sys::str` に生成されていない。

以上の結果から、issue に書かれた 100 回のリトライ上限での失敗は、この実機と条件では再現できなかった。既定のキューではエンコーダーのストレステストが `AMF_INPUT_FULL` を踏まないため、このテストは連続投入の回帰テストとして扱う。共有リトライ経路の出力回収は、`AMF_DECODER_NO_FREE_SURFACES` を発生させるデコーダーのストレステストで確認している。
