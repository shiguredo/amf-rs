# エンコーダー / デコーダーの統計値 API を追加する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-encoder-decoder-stats
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Encoder` / `Decoder` の進行状況を取得する API がなく、送信件数、出力件数、エラー件数、処理中のフレーム数を利用者が観測できない。vpl-rs、nvcodec-rs、video-toolbox-rs は `EncoderStats` / `DecoderStats` と `Counter` / `Gauge` を提供しており、amf-rs だけが欠けている。監視、`AMF_INPUT_FULL` のバックプレッシャー制御、テストのために必要である。

## 現状

- `src/encode.rs` の `Encoder` と `src/decode.rs` の `Decoder` に `stats()` がなく、統計値の型も存在しない。
- `Encoder::encode` は `SubmitInput` が成功した後に `WorkerCommand::Submit` を送り、ワーカーの `pending` と `output_buffer` が `drain_output` で対応付けられる。件数はどこにも記録されない。
- `Decoder::decode` も同じ構造で、`pending` と出力の対応付けだけを行う。
- `Encoder::reconfigure` の成功回数も記録していない。
- `AMF_VIDEO_ENCODER_STATISTICS_FEEDBACK` は実機の H.264 エンコーダーで Get / Set とも `AMF_INVALID_ARG` になった。AMF 側の PSNR / QP 統計は当てにできない。
- 兄弟の `Counter` / `Gauge` / `EncoderStats` / `DecoderStats` は 3 クレートで共通の名前と役割を持つ。

## 設計方針

- `src/stats.rs` を追加し、`Counter` と `Gauge` を実装する。内部は `Arc<AtomicU64>` とし、`Relaxed` で読み書きする。`Clone` は現在値のスナップショットを返す。
- `EncoderStats` のフィールドは `total_encode_count`、`total_output_frame_count`、`total_error_count`、`total_output_bit_count`、`total_reconfigure_count`、`in_flight_frames` とする。
- `DecoderStats` のフィールドは `total_decode_count`、`total_output_frame_count`、`total_error_count`、`in_flight_frames` とする。
- `Encoder::stats() -> &EncoderStats` と `Decoder::stats() -> &DecoderStats` を追加する。ワーカースレッドと共有するため、内部は `Arc` で保持する。
- `total_encode_count` / `total_decode_count` は AMF の `SubmitInput` が受理され、ワーカーへの送信が成功した時点で加算する。
- `in_flight_frames` は上記の受理時点で増やし、ハンドラーに結果 (`Ok` / `Err` のどちらも) を渡した時点で減らす。
- `total_output_frame_count` は `Ok` を渡した時点、`total_error_count` は `Err` を渡した時点で加算する。
- `total_output_bit_count` は `EncodedFrame::buffer()` のサイズを加算する。
- `total_reconfigure_count` は `Encoder::reconfigure` が成功した時点で加算する。
- AMF の `STATISTICS_FEEDBACK` は使わず、PSNR / QP などの画質統計を返さないことを doc に書く。
- カウンタの更新位置を `encode()` / `decode()` / ワーカー / `drain_output` のどこかに一本化し、二重加算を防ぐ。
- 実機テストを追加する。N フレーム投入後の各カウンタ、`finish()` 後の `in_flight_frames` が 0、エラー通知時の `total_error_count`、`reconfigure` 成功回数を検証する。

## 完了条件

- `Counter` / `Gauge` / `EncoderStats` / `DecoderStats` が公開され、`Encoder::stats()` / `Decoder::stats()` から取得できる。
- エンコード / デコードの送信件数、出力件数、エラー件数、出力ビット数、in-flight、reconfigure 回数が実装と一致する。
- `finish()` 後に `in_flight_frames` が 0 になる。
- AMF の `STATISTICS_FEEDBACK` を使わない理由と返さない統計の範囲が doc に書かれている。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
