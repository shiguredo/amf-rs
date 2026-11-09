# VP9 デコードに対応する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-vp9-decode
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

WebRTC や Sora で使われる VP9 のビットストリームを amf-rs でデコードできない。vpl-rs、nvcodec-rs、video-toolbox-rs は 3 クレートとも VP9 のデコードに対応している。AMF の `AMFVideoDecoderHW_VP9` は実機で `CreateComponent` と `Init(NV12, 0, 0)` が成功しており、対応しない理由がない。

## 現状

- `src/decode.rs` の `DecoderCodec` は `H264` / `Hevc` / `Av1` の 3 種類だけである。
- `src/codec_info.rs` の `VideoCodecType` も 3 種類で、`VideoCodecType::all()` と `supported_codecs` は 3 件を返す。
- `tests/test_codec_info.rs` は `supported_codecs` が 3 件であることを前提にしている。
- README と `skills/shiguredo-amf/SKILL.md` の対応コーデック表にも VP9 がない。
- AMF に VP9 エンコーダーはないため、エンコードは追加しない。

## 設計方針

- `DecoderCodec::Vp9` を追加し、`Decoder::new` で `sys::str::AMFVideoDecoderHW_VP9` を使う。
- `VideoCodecType::Vp9` を追加し、`VideoCodecType::all()` と `supported_codecs` に含める。エンコード非対応として `EncodingProfiles::None` を返す。
- `supported_codecs` の並び順を決める。既存の 3 件の順序を維持し、VP9 を末尾に追加する。
- VP9 のデコードに必要な初期化オプション (出力フォーマットなど) を確認する。0023 の安全 API と 0042 の出力フォーマット選択と整合させる。
- テスト用の VP9 ビットストリームを用意する。固定の小さなストリームを `tests/data/` に置くか、`shiguredo_libvpx` を dev-dependency にして生成する。video-toolbox-rs は `shiguredo_libvpx` を使って VP9 のラウンドトリップテストを行っている。
- 実機テストを追加する。VP9 ストリームをデコードしてフレームが出力されること、幅・高さが正しいこと、`supported_codecs` が VP9 を含むこと、VP9 のエンコードが非対応として返ることを検証する。
- README と `SKILL.md` の対応コーデック表を更新する。

## 完了条件

- `DecoderCodec::Vp9` で VP9 をデコードできる。
- `VideoCodecType::Vp9` が `supported_codecs` に含まれ、エンコード非対応として扱われる。
- `tests/test_codec_info.rs` が 4 コーデックを前提に更新される。
- 実機で VP9 のデコードを検証するテストが追加され、`cargo test --workspace` が通る。
- README と `SKILL.md` の対応コーデック表が実装と一致する。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
