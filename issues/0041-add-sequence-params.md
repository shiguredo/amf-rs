# エンコーダーから SPS / PPS / VPS を取得できるようにする

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-sequence-params
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

MP4 の avcC / hvcC / av1C や RTP の fmtp を組み立てるには、SPS / PPS / VPS などのパラメータセットが必要である。vpl-rs と nvcodec-rs は `Encoder::get_sequence_params()` を、video-toolbox-rs は `EncodedFrame` の `sps_list` / `pps_list` / `vps_list` を提供している。amf-rs はビットストリーム全体しか返さず、利用者が自前で解析する必要がある。

## 現状

- `src/encode.rs` の `EncodedFrame::buffer()` はビットストリーム全体の `Buffer` を返す。SPS / PPS / VPS を切り出す API はない。
- `src/encode.rs` は `AMF_VIDEO_ENCODER_EXTRADATA`、`AMF_VIDEO_ENCODER_HEVC_EXTRADATA`、`AMF_VIDEO_ENCODER_AV1_EXTRA_DATA` を読んでいない。
- `src/amf.rs` の `PropertyStorageFuncs` は `SetProperty` だけを保持し、`GetProperty` を持たない。`AMF_VARIANT_INTERFACE` から `AMFBuffer` を取り出すヘルパーもない。
- 実機では `EXTRADATA` が `Init` 直後から取得でき、H.264 は 25 バイト (SPS + PPS)、HEVC は 92 バイト (VPS + SPS + PPS)、AV1 の `AMF_VIDEO_ENCODER_AV1_EXTRA_DATA` は 15 バイト (Sequence Header OBU) だった。

## 設計方針

- `Encoder::get_sequence_params() -> Result<Option<SequenceParams>, Error>` を追加する。
- `SequenceParams` は `vps: Option<Vec<u8>>`、`sps: Vec<u8>`、`pps: Option<Vec<u8>>` を持つ。AV1 は `sps` に Sequence Header OBU を入れる (vpl-rs と同じ表現)。
- H.264 / HEVC の `EXTRADATA` は Annex B (スタートコード付き) で返るため、NAL ユニットに分割し、スタートコードを除去して格納する。H.264 は SPS / PPS、HEVC は VPS / SPS / PPS を NAL ユニット種別で判別する。
- AV1 の `AMF_VIDEO_ENCODER_AV1_EXTRA_DATA` は OBU のまま返す。OBU の分割は行わず、`sps` に格納する。
- `PropertyStorage` に `get_property_int64` などの取得ヘルパーを追加し、`AMF_VARIANT_INTERFACE` から `AMFBuffer` を安全に取り出す。`GetProperty` が返すインターフェースの参照カウント (Acquire / Release) を確認し、二重解放を避ける。
- `EXTRADATA` が未設定、空、または取得できない場合は `Ok(None)` を返す。
- `encode()` と同じスレッドで呼ぶこと、`reconfigure` で内容が変わり得ることを doc に書く。
- 実機テストを追加する。H.264 / HEVC / AV1 で空でないパラメータセットが返ること、スタートコードが含まれないこと、`reconfigure` 後に内容が変わること、`finish()` 後でも取得できることを検証する。

## 完了条件

- `Encoder::get_sequence_params()` が公開され、`SequenceParams` を返す。
- H.264 / HEVC / AV1 で SPS / PPS / VPS または Sequence Header OBU が取得できる。
- スタートコードの扱いと AV1 の表現が doc に書かれている。
- 取得できない場合に `Ok(None)` を返す。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
