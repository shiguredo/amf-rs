# デコーダーの低遅延・サーフェスプール設定に対応する

- Created: 2026-11-10
- Completed: 2026-11-22
- Branch: feature/add-decoder-latency-options
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`DecoderConfig` はコーデック種別しか持たず、低遅延モード、リオーダーモード、サーフェスプールサイズを設定できない。RTC ではデコーダーの遅延がそのまま通話遅延になるため、B フレームの並べ替えを無効にする低遅延モードや、サーフェスプールのサイズを調整する手段が必要である。vpl-rs は `DecoderConfig::async_depth` を、nvcodec-rs は `max_display_delay` と `max_num_decode_surfaces` を持っている。

## 現状

- `src/decode.rs` の `DecoderConfig` は `codec` だけを持つ。
- `Decoder::new` は `component.init(AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, 0, 0)` の前に、デコーダー固有のプロパティを設定していない。
- `src/amf.rs` の `PropertyStorage` には int64 プロパティを設定する `set_property_int64` があるが、デコーダー側からは使われていない。
- AMF は `AMF_VIDEO_DECODER_LOW_LATENCY`、`AMF_VIDEO_DECODER_REORDER_MODE`、`AMF_VIDEO_DECODER_SURFACE_POOL_SIZE`、`AMF_VIDEO_DECODER_DPB_SIZE` を持つ。`AMF_VIDEO_DECODER_DPB_SIZE` は Linux の AMF ランタイム 1.5.3.0 では設定が `AMF_ACCESS_DENIED` になるため対象外とする。
- H.264 / HEVC / AV1 の各デコーダーで同じプロパティ名が受け付けられるかは未確認である。

## 設計方針

- `DecoderConfig` に `low_latency: Option<bool>`、`reorder_mode: Option<DecoderReorderMode>`、`surface_pool_size: Option<u32>` を追加する。
- `DecoderReorderMode` は AMF の `AMF_VIDEO_DECODER_MODE_ENUM` を写像した enum とする。
- `low_latency = true` と `reorder_mode` の組み合わせ、B フレーム入力時の出力順の扱い、出力フレームの PTS との関係を doc に書く。
- `surface_pool_size` の 0 の意味 (AMF の既定) と、値の検証を決める。
- 各コーデックでプロパティが受け付けられるか実機で確認する。未対応のコーデックではエラーにするか doc で明示する。
- DPB サイズは AMF ランタイムが設定を許可しないため公開 API に含めない。この制約は doc に明記する。
- 設定値が出力フレームの欠落や重複を引き起こさないことを確認する。0026 の解像度変化と併用した場合の挙動も確認する。
- 実機テストを追加する。低遅延設定でデコードできること、出力フレーム数が設定なしの場合と変わらないこと、未対応のコーデックでの挙動を検証する。
- README と `skills/shiguredo-amf/SKILL.md` に設定項目を追加する。

## 完了条件

- `DecoderConfig` で低遅延モード、リオーダーモード、サーフェスプールサイズを指定できる。
- 各コーデックでのプロパティの受け付け可否が実機で確認され、未対応時の挙動が doc に書かれている。
- DPB サイズを公開 API に含めない理由が doc に書かれている。
- 低遅延設定で出力フレームの欠落・重複がないことを検証するテストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。

## 解決方法

- `DecoderConfig` に `low_latency` / `reorder_mode` / `surface_pool_size` を追加し、`DecoderReorderMode` と `DecoderConfig::new` を追加した。
- `low_latency` は `AMF_VIDEO_DECODER_LOW_LATENCY`、`reorder_mode` は `AMF_VIDEO_DECODER_REORDER_MODE`、`surface_pool_size` は `AMF_VIDEO_DECODER_SURFACE_POOL_SIZE` に `Init` 前に設定するようにした。
- 解像度変化で `Terminate` / `Init` する場合も、作成時と同じ設定を再適用するようにした。
- `DecoderReorderMode` は AMF の `AMF_VIDEO_DECODER_MODE_ENUM` に写像した。`LowLatency` はフレームの並べ替えを伴わないストリームを前提とし、並べ替えを伴うストリームの出力順は保証せず、提示順の復元も行わないことを doc に書いた。
- `low_latency` と `reorder_mode` の役割の違い、出力フレームの PTS と `user_data` の対応付けを README / `skills/shiguredo-amf/SKILL.md` に書いた。
- H.264 / HEVC / AV1 / VP9 の各デコーダーで設定が受け付けられることを実機テストで確認した。H.264 / HEVC / AV1 / VP9 の低遅延デコードでフレームの欠落・重複がないことを確認するテストと、解像度変化を挟んでも設定を指定したまま全フレームが出力されることを確認するテストを追加した。
- `src/decode.rs` の単体テストで、`Regular` / `Compliant` / `LowLatency` が AMF の列挙値に正しく写像されることを確認した。
- DPB サイズは Linux の AMF ランタイム 1.5.3.0 で `AMF_ACCESS_DENIED` になり設定できないため、公開 API に含めない判断とし、その理由を doc に書いた。
- README / `skills/shiguredo-amf/SKILL.md` / `CHANGES.md` を更新し、`CHANGES.md` の `## develop` に `[CHANGE]` と `[ADD]` を追記した。
- AMD Radeon 780M / AMF ランタイム 1.5.3.0 の実機で `cargo test --workspace` が成功した。`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace --no-run` も通した。
