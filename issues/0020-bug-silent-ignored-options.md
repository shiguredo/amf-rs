# 公開フィールドが黙って無視される組み合わせを修正する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
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
- `EncodeOptions.frame_type` は `I` / `P` / `B` を `AMF_FORCE_PICTURE_TYPE` にマッピングするか、`IDR` 以外をエラーにする。少なくとも doc に「現状 IDR のみ有効」と明記する。
- `Encoder::new` / `Encoder::reconfigure` で未対応の組み合わせを検証し、黙って無視しない。
- テストは、実際の出力ビットストリームの GOP 構造やピクチャタイプを確認するか、エラーが返ることを確認する。フレーム数だけの検証にしない。
- README と `skills/shiguredo-amf/SKILL.md` の対応表を実装に合わせる。

## 完了条件

- AV1 の初期化と `reconfigure` で `qpb` の扱いが一致する。
- HEVC で未対応のフィールドを指定した場合に、黙って無視されずエラーになるか、doc で明示される。
- `frame_type` の未対応フラグが、マッピングされるかエラーになる。
- テストが実際の効果またはエラーを検証する。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` または `[CHANGE]` を追記する。
