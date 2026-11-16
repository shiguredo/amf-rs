# 公開フィールドが黙って無視される組み合わせを修正する

- Created: 2026-11-10
- Completed: 2026-11-16
- Branch: feature/fix-silent-ignored-options
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`EncoderConfig` / `ReconfigureParams` / `EncodeOptions` の公開フィールドのうち、特定のコーデックや操作で黙って無視されるものがある。利用者は指定した値が反映されたと誤解する。CODEBASE.md は「ユーザーが渡す値は検証すること」と定めており、未対応の組み合わせはエラーにするか、少なくとも doc で明示する必要がある。

## 現状

- `EncoderConfig.qpb` は AV1 の初期化では設定されないが、`Encoder::reconfigure` では `Q_INDEX_INTER_B` として設定される。初期化と動的更新で扱いが非対称である。
- `ReconfigureParams.qpb` と `ReconfigureParams.gop_pic_size` は HEVC では無視される。AMF の HEVC には B フレーム用の QP プロパティがなく、GOP サイズは Init 前の静的プロパティである。エラーも警告も出ない。
- `EncodeOptions.frame_type` は IDR ビットだけが `Encoder::encode` で見られており、`I` / `P` / `B` / `REF` を指定しても何も起きない。doc は「強制するフレームタイプ」と読める。
- `tests/test_roundtrip.rs` の `test_reconfigure_hevc_ignores_qpb_and_gop` はデコードフレーム数だけを確認しており、値が無視されたこと自体を検証していない。
- README と `skills/shiguredo-amf/SKILL.md` の reconfigure の対応表は、HEVC の `qpb` / `gop_pic_size` が無視されることを明記しているが、初期化時の `qpb` の非対称は書かれていない。

## 設計方針

- AV1 の初期化でも `qpb` を `AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTER_B` に設定し、`reconfigure` と対称にする。
- HEVC の `qpb` / `gop_pic_size` は、未対応であることを `Error` として返す。難しければ `ReconfigureParams` をコーデック別の型に分ける。
- `EncodeOptions.frame_type` は `IDR` / `I` / `P` を優先順位付きでマッピングし、`B` / `REF`、AV1 の `P`、未定義のビットは無視する。無視する値は doc に明記する。
- `Encoder::new` / `Encoder::reconfigure` で未対応の組み合わせを検証し、黙って無視しない。
- テストは、実際の出力ビットストリームの GOP 構造やピクチャタイプを確認するか、エラーが返ることを確認する。フレーム数だけの検証にしない。
- README と `skills/shiguredo-amf/SKILL.md` の対応表を実装に合わせる。

## 完了条件

- AV1 の初期化と `reconfigure` で `qpb` の扱いが一致する。
- HEVC で未対応のフィールドを指定した場合に、黙って無視されずエラーになるか、doc で明示される。
- `frame_type` の未対応フラグの扱い (マッピングまたは無視) が doc に明記される。
- テストが実際の効果またはエラーを検証する。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` または `[CHANGE]` を追記する。

## 解決方法

- `EncoderConfig::qpb` は AV1 の初期化でも `AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTER_B` に設定し、`Encoder::reconfigure` と対称にした。
- HEVC では B フレーム用の QP プロパティと初期化後の GOP サイズ変更が存在しないため、`EncoderConfig::qpb` は `Encoder::new` が、`ReconfigureParams::qpb` / `ReconfigureParams::gop_pic_size` は `Encoder::reconfigure` がエラーを返すようにした。
- `Encoder::encode` で `EncodeOptions::frame_type` の指定から `IDR` / `I` / `P` を `IDR`、`I`、`P` の順に優先して 1 つ選び、コーデックに応じた AMF の強制ピクチャタイプへ変換するようにした。H.264 と HEVC は `FORCE_PICTURE_TYPE` の IDR / I / P、AV1 は `FORCE_FRAME_TYPE` の KEY / INTRA_ONLY を使う。
- `B` / `REF`、AV1 の `P`、未定義のビットはエラーにせず無視するようにした。フラグはコーデックごとのヒントであり、対応しない値をエラーにすると vpl-rs 風の共通指定が使えなくなるため。
- `EncodeOptions::frame_type` と `frame_type` モジュール、`Encoder::encode` の doc に、優先順位、無視するフラグ、先頭フレームおよび GOP の境界では IDR が優先される場合があることを明記した。
- `tests/test_roundtrip.rs` に、H.264 / HEVC の `I` / `P` 強制と AV1 の `I` 強制が実際の出力ピクチャタイプに反映されること、複数指定時の優先順位、`B` / `REF` / AV1 の `P` / 未定義ビットが無視されることを検証するテストを追加した。
- README と `skills/shiguredo-amf/SKILL.md` の `frame_type` の説明を実装に合わせ、`CHANGES.md` の `## develop` に `[FIX]` を追記した。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`、`cargo test --workspace` が AMD GPU 環境で通ることを確認した。
