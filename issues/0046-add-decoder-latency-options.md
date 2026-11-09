# デコーダーの低遅延・DPB 設定に対応する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-decoder-latency-options
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`DecoderConfig` はコーデック種別しか持たず、低遅延モード、DPB サイズ、サーフェスプールサイズを設定できない。RTC ではデコーダーの遅延がそのまま通話遅延になるため、B フレームの並べ替えを無効にする低遅延モードや、DPB のサイズを調整する手段が必要である。vpl-rs は `DecoderConfig::async_depth` を、nvcodec-rs は `max_display_delay` と `max_num_decode_surfaces` を持っている。

## 現状

- `src/decode.rs` の `DecoderConfig` は `codec` だけを持つ。
- `Decoder::new` は `component.init(AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, 0, 0)` の前に、デコーダー固有のプロパティを設定していない。
- `src/amf.rs` の `PropertyStorage` には int64 プロパティを設定する `set_property_int64` があるが、デコーダー側からは使われていない。
- AMF は `AMF_VIDEO_DECODER_LOW_LATENCY`、`AMF_VIDEO_DECODER_REORDER_MODE`、`AMF_VIDEO_DECODER_SURFACE_POOL_SIZE`、`AMF_VIDEO_DECODER_DPB_SIZE` を持つ。
- H.264 / HEVC / AV1 の各デコーダーで同じプロパティ名が受け付けられるかは未確認である。

## 設計方針

- `DecoderConfig` に `low_latency: Option<bool>`、`reorder_mode: Option<DecoderReorderMode>`、`surface_pool_size: Option<u32>`、`dpb_size: Option<u32>` を追加する。
- `DecoderReorderMode` は AMF の `AMF_VIDEO_DECODER_MODE_ENUM` を写像した enum とする。
- `low_latency = true` と `reorder_mode` の組み合わせ、B フレーム入力時の出力順の扱い、出力フレームの PTS との関係を doc に書く。
- `surface_pool_size` / `dpb_size` の 0 の意味 (AMF の既定) と、値の検証を決める。
- 各コーデックでプロパティが受け付けられるか実機で確認する。未対応のコーデックではエラーにするか doc で明示する。
- 設定値が出力フレームの欠落や重複を引き起こさないことを確認する。0026 の解像度変化と併用した場合の挙動も確認する。
- 実機テストを追加する。低遅延設定でデコードできること、出力フレーム数が設定なしの場合と変わらないこと、DPB サイズを変えても破綻しないこと、未対応のコーデックでの挙動を検証する。
- README と `skills/shiguredo-amf/SKILL.md` に設定項目を追加する。

## 完了条件

- `DecoderConfig` で低遅延モード、リオーダーモード、サーフェスプールサイズ、DPB サイズを指定できる。
- 各コーデックでのプロパティの受け付け可否が実機で確認され、未対応時の挙動が doc に書かれている。
- 低遅延設定で出力フレームの欠落・重複がないことを検証するテストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
