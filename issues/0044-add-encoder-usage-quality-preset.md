# エンコーダーの用途と品質プリセットを設定できるようにする

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-encoder-usage-quality-preset
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

現在のエンコーダーは `USAGE` を 0 (transcoding) に固定し、`QUALITY_PRESET` を設定していない。RTC の低遅延、録画の高画質、カメラ入力の用途を切り替えられず、利用者は AMF の既定値に依存するしかない。vpl-rs は `target_usage`、nvcodec-rs は `Preset` と `TuningInfo`、video-toolbox-rs は速度優先とリアルタイムの設定を持っている。amf-rs にも同等の設定が必要である。

## 現状

- `src/encode.rs` の `set_h264_properties`、`set_hevc_properties`、`set_av1_properties` が `AMF_VIDEO_ENCODER_USAGE`、`AMF_VIDEO_ENCODER_HEVC_USAGE`、`AMF_VIDEO_ENCODER_AV1_USAGE` に 0 を設定している。
- `AMF_VIDEO_ENCODER_QUALITY_PRESET`、`AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET`、`AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET` は設定していない。
- `EncoderConfig` に用途や品質プリセットのフィールドがない。
- 実機で H.264 の品質プリセット 0..3 と usage 0..5、HEVC の 0 / 5 / 10 / 15、AV1 の 0 / 30 / 70 / 100 がすべて `Init` 成功した。
- 値はコーデックごとに異なる。AV1 の usage は low latency と ultra low latency の順序が H.264 / HEVC と逆である。

## 設計方針

- `EncoderUsage` enum を追加する。値は `Transcoding`、`UltraLowLatency`、`LowLatency`、`Webcam`、`HighQuality`、`LowLatencyHighQuality` とする。
- `QualityPreset` enum を追加する。値は `Speed`、`Balanced`、`Quality`、`HighQuality` とする。
- `EncoderConfig` に `usage: Option<EncoderUsage>` と `quality_preset: Option<QualityPreset>` を追加する。既定は `None` とし、指定時だけ AMF のプロパティを設定する。既存の `USAGE = 0` 固定の挙動は変えない。
- コーデック別の値への写像を関数にまとめ、数値のマジックナンバーを散らさない。0033 のレート制御 enum 化と整合させる。
- `usage` と `quality_preset` の組み合わせ、およびレート制御モードとの関係を doc に書く。AMF は usage でパラメータ一式を構成するため、明示的に設定した他のプロパティとの優先順位を確認する。
- 実機テストを追加する。各コーデックで全ての `EncoderUsage` と `QualityPreset` を `Init` できること、可能なら数フレームのエンコードまで通ることを検証する。
- README と `skills/shiguredo-amf/SKILL.md` に設定項目を追加する。

## 完了条件

- `EncoderConfig::usage` と `EncoderConfig::quality_preset` で設定できる。
- コーデックごとの値の写像が実装され、H.264 / HEVC / AV1 で `Init` が成功する。
- 未指定時に既存の挙動が変わらない。
- usage と quality preset の関係、および他のレート制御設定との優先順位が doc に書かれている。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
