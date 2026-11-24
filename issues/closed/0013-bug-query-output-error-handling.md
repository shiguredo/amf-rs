# `QueryOutput` のエラーと `finish` 時の未マッチ出力を通知する

- Created: 2026-11-10
- Completed: 2026-11-16
- Branch: feature/fix-query-output-error-handling
- Polished: {YYYY-MM-DD}
- Refreshed: 2026-11-15
- Reporter: @melpon

## 目的

`QueryOutput` が `AMF_OK` / `AMF_REPEAT` / `AMF_EOF` 以外の結果を返した場合、現在はその理由がハンドラーに通知されない。コンポーネントごとの症状は次のとおり。

- デコーダーは想定外の結果を `DrainStatus::Drained` として扱い、`pending` が残っていてもポーリングを続ける。`finish()` にタイムアウトがないため、`AMF_EOF` が来ないと永久に返らない。
- エンコーダーは想定外の結果を `break` で捨てる。`pending` が減らないため、`finish()` が 5 秒後に `Finish wait timed out` を返す。

また、エンコーダーのワーカーは `pending` が空になった時点で終了するため、`output_buffer` に残った未マッチ出力を無通知で破棄する。

`QueryOutput` の終端エラーを通知し、ワーカーがタイムアウトなしで終了できるようにするのが目的である。デバイスが応答しない、ブロックする、不定値を返すといった異常は対象外とする。

## 現状

- `src/decode.rs` の `drain_output` は `AMF_REPEAT` / `AMF_OK` / `AMF_EOF` を分類するが、それ以外の結果は `DrainStatus::Drained` として返すだけで、エラーを通知しない。`AMF_REPEAT` でデータが非 null の場合は出力として扱う既存の挙動を持つ。
- `src/decode.rs` のワーカーは `finish` を受けた後、`AMF_EOF` が返るか、`pending` と `output_buffer` が両方空になるまでポーリングする。`Decoder::finish` は `rx.recv()` で待つためタイムアウトがなく、`QueryOutput` がエラーを返し続けて `pending` が残ると永久に返らない。
- `src/decode.rs` は `user_data` を PTS で対応付ける (`HashMap<amf_pts, PendingFrame>`)。`deliver_buffered` / `flush_buffered_outputs` / `report_unmatched_inputs` が未マッチの出力・入力を `Err` として通知する。この経路はテスト済みである。ただし `QueryOutput` 自体のエラーは `output_buffer` に積まれない。
- `src/encode.rs` の `drain_output` は `AMF_REPEAT` / `AMF_EOF` 以外の非 OK と null データを `break` で捨てる。
- `src/encode.rs` のワーカー終了条件は `finish.is_some() && pending.is_empty()` だけで、`output_buffer` の残量を見ない。`Encoder::finish` は 5 秒の `recv_timeout` を持ち、タイムアウト時は `Finish wait timed out` を返す。残りの `pending` 数や最後の `QueryOutput` の結果は含まない。
- 両方とも、`output_buffer` は `Result<T, Error>` を保持できるが、積まれるのはフレーム抽出のエラーだけである。
- デコーダーは 1 バッファ 1 フレームを前提とし、`AMF_REPEAT` は複数フレームとしてエラーにする。並べ替えは PTS で対応付けるため、未マッチ出力の原因にはならない。

## 設計方針

- `QueryOutput` の結果を次のように分類する。判定は `AMF_OK` + null と非 OK を混ぜず、データの有無より先にステータスを見る。
  - 出力あり: `AMF_OK` または `AMF_REPEAT` でデータが非 null。`AMF_REPEAT` でデータを返す実装があるため、データがある場合は出力として扱う。エンコーダーも同じ分類を使う。
  - 進捗なし: `AMF_REPEAT` かつデータが null、または `AMF_OK` かつデータが null。出力なしとして継続する。
  - ドレイン完了: `AMF_EOF`。保持している出力を flush し、未マッチの入力を `Err` で通知してワーカーを終了する。
  - 終端エラー: 上記以外 (`AMF_RESOLUTION_CHANGED` / `AMF_RESOLUTION_UPDATED` を除く)。ハンドラーへ `Err` を通知し、残りの `pending` をすべて未処理として `Err` で通知してワーカーを終了する。`AMF_EOF` は待たない。
- `AMF_RESOLUTION_CHANGED` / `AMF_RESOLUTION_UPDATED` は解像度変化の通知であり、0026 の範囲とする。この issue では終端エラーに含めず、現状の扱いを維持する。
- 終端エラーは `finish` 中に限らず発生し得る。ワーカーはエラーを通知して終了する。進行中の `finish()` はワーカーの完了を待って戻り、以降の `encode` / `decode` / `finish` は既存どおりチャネル切断のエラーを返す。
- 終端エラーの内容はハンドラーへ通知し、`finish()` の戻り値では重複して返さない。ワーカーが既に終了している場合の `finish()` はチャネル切断のエラーを返す。
- エラーは `drain_output` の戻り値で判定し、`output_buffer` に積まない。`output_buffer` は取得済みの出力だけを保持する。
- `finish()` のタイムアウトは設けない。`Encoder::finish` の 5 秒の `recv_timeout` を削除し、`Decoder::finish` と同じくワーカーの完了まで待つ。ワーカーの panic はチャネル切断として `Err` で検知する。
- タイムアウトは採用しない。デバイスが応答しない・ブロックする・不定値を返す状態は対象外であり、タイムアウトを置いてもワーカーを停止できず、`Drop` の `join` も待ち続けるためである。
- エンコーダーの `drain_output` も `DrainStatus` を返すようにし、`finish()` でも同じ方針に揃える。`pending` が空でも `output_buffer` に残りがあれば終了せず flush する。対応する `pending` がない出力は `Err` として通知し、出力が得られなかった `pending` も `Err` として通知する。
- デコーダーの PTS ベース対応付けは維持する。終端エラーは特定の入力に対応しないため、`pending` を 1 つ消費する形にはしない。
- `QueryOutput` の結果分類を純粋関数として切り出し、実際の AMF では再現できないエラー結果が終端として分類されることを単体テストする。AMF の挙動を模したモックは使わない。
- `Encoder::finish` / `Decoder::finish` の doc に、タイムアウトせずワーカーの完了を待つこと、終端エラーがハンドラーへ通知されること、ワーカーが既に終了している場合はチャネル切断のエラーを返すことを書く。

## 完了条件

- `QueryOutput` の終端エラー (`AMF_RESOLUTION_CHANGED` / `AMF_RESOLUTION_UPDATED` を除く) がハンドラーに `Err` として通知される (エンコーダー・デコーダー両方)。
- `QueryOutput` の終端エラーでワーカーが無限にポーリングせず、`finish()` にタイムアウトを追加せずに戻る。
- `AMF_RESOLUTION_CHANGED` / `AMF_RESOLUTION_UPDATED` は終端エラーとして扱わない。
- `Encoder::finish` の 5 秒タイムアウトが削除され、ワーカーの完了まで待つ。
- `Encoder::finish` が `output_buffer` の残りを無通知で破棄しない。
- デコーダーの未マッチ出力・入力の通知が維持される。
- `QueryOutput` の結果分類の単体テストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 解決方法

- `QueryOutput` の結果を `src/query_output.rs` の `classify_query_output` で分類し、`AMF_OK` / `AMF_REPEAT` / `AMF_EOF` / 解像度変化 / 終端エラーを区別するようにした。
- エンコーダーとデコーダーのワーカーは、終端エラーをハンドラーへ `Err` として通知し、取得済みの出力と未処理の入力を通知して終了するようにした。
- エンコーダーの `finish()` は 5 秒タイムアウトをやめ、`Drain` 後に残りの出力と未処理の入力を通知してワーカーの完了まで待つようにした。デコーダーの `finish()` も同じ待ち方に揃えた。
- 出力として扱わない `QueryOutput` のデータは解放し、エンコーダーでも `AMF_REPEAT` + 非 null のデータを出力として扱うようにした。
- デコーダーの抽出エラーは対応する入力の PTS を保持し、未対応入力の通知順を入力順にした。同じ入力へのエラー二重通知も防ぐ。
- `QueryOutput` の分類テストと `CHANGES.md` の `[FIX]` を追加し、`Encoder::finish` / `Decoder::finish` の doc を現在の契約に更新した。
