# AV1 の `PictureType` 判定で `INTRA_ONLY` を P と誤報告しないようにする

- Created: 2026-11-10
- Completed: 2026-11-16
- Branch: feature/fix-av1-picture-type
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Encoder` の出力フレームの `PictureType` を取得する処理が、AV1 では 0 以外をすべて `P` として扱っている。AMF の AV1 出力フレーム種別は `KEY = 0`、`INTRA_ONLY = 1`、`INTER = 2`、`SWITCH = 3`、`SHOW_EXISTING = 4` であり、`INTRA_ONLY` (I フレーム相当) を `P` と誤報告する。`SWITCH` と `SHOW_EXISTING` も `P` ではない。

H.264 と HEVC の判定には未知値のフォールバック (`PictureType::Unknown`) があるが、AV1 にはない。

## 現状

- `src/encode.rs` の `get_output_picture_type` が `CodecConfig::Av1` の場合に `0 => PictureType::Idr`、それ以外は `PictureType::P` としている。
- `AV1` の `AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM` の全値をマッピングしていない。
- `PictureType` のテストは、強制 IDR 時の KEY だけを確認しており、`INTRA_ONLY` / `SWITCH` / `SHOW_EXISTING` を検証していない。

## 設計方針

- `INTRA_ONLY` を `PictureType::I`、`INTER` を `PictureType::P` にマッピングする。
- `SWITCH` と `SHOW_EXISTING` は `PictureType::Unknown` にする。必要なら `PictureType` に variant を追加するかを検討するが、既存の H.264 / HEVC との整合を優先する。
- 未知の値も `PictureType::Unknown` にする。
- 取得した `AMFVariantStruct` の型が `AMF_VARIANT_INT64` であることを確認する。型が異なる場合は `PictureType::Unknown` にする。
- AV1 の `INTRA_ONLY` を含むテストを追加する。テスト用のビットストリームで `INTRA_ONLY` を発生させるのが難しい場合は、マッピングを純粋関数に切り出して単体テストする。

## 完了条件

- AV1 の `KEY` / `INTRA_ONLY` / `INTER` が `Idr` / `I` / `P` に正しくマッピングされる。
- `SWITCH` / `SHOW_EXISTING` と未知の値が `Unknown` になる。
- マッピングのテストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 解決方法

- `src/encode.rs` の `picture_type_from_output_value` で、AV1 の `AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM` の全値をマッピングした。`KEY` を `PictureType::Idr`、`INTRA_ONLY` を `PictureType::I`、`INTER` を `PictureType::P`、`SWITCH` / `SHOW_EXISTING` と未知の値を `PictureType::Unknown` にする。
- H.264 / HEVC / AV1 の出力種別値は、bindgen が生成した定数から作るローカル定数を使うようにした。
- `tests/test_roundtrip.rs` の `test_av1_picture_types` で、実際の AMF を使って先頭フレームが `Idr`、後続の `INTER` フレームが `P` になることを検証した。
- `src/encode.rs` の `test_picture_type_from_output_value_av1_edge_cases` で、公開 API では生成を制御できない `INTRA_ONLY` / `SWITCH` / `SHOW_EXISTING` と未知の値を検証した。
- `AMFVariantStruct` の型確認は、CODEBASE.md の「AMF が返すメタデータは仕様どおり信頼し、健全性の検証をしない」に従い行っていない。AMF の仕様で出力フレーム種別のプロパティは `amf_int64` と定義されているため、そのまま `int64Value` を読む。
- `CHANGES.md` の `## develop` に `[FIX]` を追記した。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` が AMD GPU 環境で通ることを確認した。
