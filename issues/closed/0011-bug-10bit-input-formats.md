# 10bit 系の入力フォーマットと Main10 プロファイルで初期化できるようにする

- Created: 2026-11-10
- Completed: 2026-11-14
- Branch: feature/fix-10bit-input-formats
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`FrameFormat` は P010 / P012 / P016 / Y210 / Y410 / Y416 を公開し、`HevcProfile::Main10` も公開しているが、`AMF_*_COLOR_BIT_DEPTH` を設定していない。AMF の既定は 8bit であるため、これらの入力フォーマットや Main10 を指定すると `AMFComponent::Init` が `AMF_INVALID_FORMAT` で失敗する。

README と `skills/shiguredo-amf/SKILL.md` は 15 種類の入力フォーマットを無条件に対応として掲載しているが、実機で初期化できるのは 8bit の 7 種類だけである。10bit の入出力を扱えるようにし、対応表を実態に合わせる。

## 現状

- `src/encode.rs` の `FrameFormat` は P010 / P012 / P016 / Y210 / Y410 / Y416 を AMF のサーフェスフォーマットに写像している。
- エンコーダーのプロパティ設定 (`set_h264_properties` / `set_hevc_properties` / `set_av1_properties`) に `AMF_VIDEO_ENCODER_COLOR_BIT_DEPTH`、`AMF_VIDEO_ENCODER_HEVC_COLOR_BIT_DEPTH`、`AMF_VIDEO_ENCODER_AV1_COLOR_BIT_DEPTH` の設定がない。
- 実機確認 (H.264 High、320x240) では、Uyvy / P010 / P012 / P016 / Y210 / Ayuv / Y410 / Y416 が `AMF_INVALID_FORMAT` になった。成功したのは NV12 / YV12 / I420 / BGRA / ARGB / RGBA / YUY2 の 7 種類である。
- HEVC Main10 + P010 も `AMF_INVALID_FORMAT` になり、HEVC Main10 + NV12 は成功した。
- テストは 8bit の 7 種類だけで、残りは `unimplemented!` により未検証である。

## 設計方針

- `FrameFormat` とプロファイルから期待するビット深度を決め、対応する `AMF_*_COLOR_BIT_DEPTH` を設定する。8bit は `AMF_COLOR_BIT_DEPTH_8`、10bit は `AMF_COLOR_BIT_DEPTH_10` とする。
- AMF に対応するビット深度がない P012 / P016 は、`Encoder::new` で明確なエラーを返す。
- H.264 の 10bit は AMF のプロファイルに存在しないため、`Encoder::new` でエラーを返す。
- Main10 と 8bit 入力の組み合わせを許可するかエラーにするかを決め、doc に書く。
- UYVY / AYUV などハードウェア依存のフォーマットは、README に「GPU と AMF の実装に依存する」と明記する。能力照会 API の追加は別 issue とする。
- P010 + HEVC Main10、P010 + AV1、Y210 / Y410 の初期化テストを追加する。ハードウェアが非対応の場合はテストをスキップするのではなく、`Encoder::new` が返すエラーを検証する。

## 完了条件

- 10bit 系の入力フォーマットと Main10 が、ハードウェアの対応範囲で初期化・エンコードできる。
- 非対応の組み合わせは `AMF_INVALID_FORMAT` ではなく、`Encoder::new` の明確なエラーになる。
- README と `skills/shiguredo-amf/SKILL.md` の対応表が実態と一致する。
- テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に変更を追記する。

## 解決方法

- `FrameFormat::color_bit_depth` で入力フォーマットのビット深度を決め、`Encoder::new` で `AMF_VIDEO_ENCODER_COLOR_BIT_DEPTH` / `AMF_VIDEO_ENCODER_HEVC_COLOR_BIT_DEPTH` / `AMF_VIDEO_ENCODER_AV1_COLOR_BIT_DEPTH` を設定する
- H.264 は 8bit 入力のみ、HEVC と AV1 は 8bit と 10bit の入力に対応する
- 入力フォーマットとプロファイルの組み合わせは基本的に AMF に任せるが、HEVC の 10bit 入力と Main プロファイル (プロファイル未指定を含む) の組み合わせは、AMF が Main プロファイルに適合しない 10bit ストリームを生成するため、`Encoder::new` がエラーを返す
- P012 / P016 / Y416 は AMF の `AMF_COLOR_BIT_DEPTH_ENUM` が扱えないため色深度を設定せず、AMF の `AMFComponent::Init` に任せる
- UYVY / AYUV / Y210 / Y410 は GPU と AMF の実装に依存するフォーマットとして README と `skills/shiguredo-amf/SKILL.md` に明記する
- `tests/test_encode.rs` に以下を追加する
  - P010 + HEVC Main10 / P010 + AV1 の初期化とエンコード
  - HEVC の 10bit 入力と Main / プロファイル未指定の組み合わせの拒否
  - Y210 / Y410 の初期化。非対応の場合は `AMFComponent::Init` が返す `AMF_INVALID_FORMAT` を検証する
- `cargo test --workspace` を実行し、実機 (shiguredo-amd-um790) で全テストが通ることを確認する

当初の完了条件にあった「非対応の組み合わせは `AMF_INVALID_FORMAT` ではなく `Encoder::new` の明確なエラーにする」は採用しない。
ただし、AMF が Main プロファイルのまま 10bit を出力する HEVC の組み合わせだけは、ライブラリ側で拒否する。
