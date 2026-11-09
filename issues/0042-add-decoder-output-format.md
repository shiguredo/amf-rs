# デコード出力フォーマットの選択と表示サイズの取得に対応する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-decoder-output-format
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Decoder` は常に NV12 で初期化するため、HEVC Main10 や AV1 10bit のデコード結果を 10bit のまま取得できない。また `DecodedFrame` に表示サイズとピクセルフォーマットの accessor がなく、利用者は coded size から表示サイズを推測する必要がある。vpl-rs はストリームのネイティブ形式 (P010 / AYUV / Y410) と表示サイズを返し、video-toolbox-rs は `PixelFormat` で出力形式を選べる。amf-rs にも同等の機能が必要である。

## 現状

- `src/decode.rs` の `Decoder::new` は `component.init(AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, 0, 0)` を固定で呼ぶ。
- `DecoderConfig` は `codec` だけを持ち、出力フォーマットを指定できない。
- `DecodedFrame` は `surface()` と `user_data()` だけを公開し、幅・高さ・ピクセルフォーマットを返さない。`Plane::get_width` / `get_height` は coded size であり、表示サイズ (crop) ではない。
- AMF の HEVC / AV1 デコーダーは `Init` が NV12 / P010 / P012 / P016 / YUV420P で成功した。
- `AMF_VIDEO_DECODER_CURRENT_SIZE` と `AMF_VIDEO_DECODER_OUTPUT_FORMAT` で表示サイズと検出フォーマットを取得できる見込みである。
- `issues/0023-change-safe-frame-api.md` はフレームデータの安全な読み書きを扱うが、出力フォーマットの選択と表示サイズの取得は扱っていない。

## 設計方針

- `DecoderConfig` に `output_format: DecoderOutputFormat` を追加する。既定は `Nv12` とし、既存の挙動を変えない。
- `DecoderOutputFormat` は少なくとも `Nv12`、`P010`、`P012`、`P016`、`Yuv420p` を持つ。`AMF_SURFACE_FORMAT` への写像を持つ。既存の `FrameFormat` を再利用するか、デコード専用の enum にするかを決める。再利用する場合は、エンコード専用のフォーマットを含めないようにする。
- `DecodedFrame` に `width()`、`height()`、`format()` を追加する。表示サイズは `AMF_VIDEO_DECODER_CURRENT_SIZE` を優先し、取得できない場合はプレーンの幅・高さにフォールバックする。フォーマットは要求値ではなく、`AMF_VIDEO_DECODER_OUTPUT_FORMAT` が返す実効値を使う。
- `Init` に失敗した場合は、要求したフォーマットと AMF のエラーを含むメッセージを返す。
- 出力 Surface のピッチ、プレーンサイズ、`convert(AMF_MEMORY_HOST)` の扱いを 0023 の安全 API と整合させる。
- 実機テストを追加する。H.264 1080p で表示サイズが 1920x1080 になること (coded size は 1088 になり得る)、HEVC Main10 と AV1 10bit で P010 出力が得られること、要求したフォーマットと実効値が一致しない場合の扱いを検証する。10bit のテストストリームは 0011 の 10bit エンコード対応後に生成するか、固定ストリームを用意する。
- README と `skills/shiguredo-amf/SKILL.md` の「デコード出力は NV12」を更新する。

## 完了条件

- `DecoderConfig::output_format` でデコード出力フォーマットを選択できる。
- `DecodedFrame::width()` / `height()` / `format()` が表示サイズと実効フォーマットを返す。
- 既定の `Nv12` で既存の動作が変わらない。
- HEVC Main10 と AV1 10bit の P010 出力を検証する実機テストが追加される。
- README と `SKILL.md` のデコード出力の記述が実装と一致する。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
