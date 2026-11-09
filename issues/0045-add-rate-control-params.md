# レート制御パラメータを拡充する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-rate-control-params
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

現在の `EncoderConfig` で設定できるレート制御の項目は target / max kbps、QPI / QPP / QPB、GOP だけである。VBV バッファ、Min / Max QP、QVBR の品質レベル、フレームスキップなどを設定できず、画質、遅延、帯域の制御ができない。vpl-rs は `buffer_size_in_kb`、`initial_delay_in_kb`、`qvbr_quality`、`look_ahead_depth` などを持ち、AMF 自身も同等のプロパティを持っている。

## 現状

- `src/encode.rs` の `EncoderConfig` は `target_kbps`、`max_kbps`、`qpi`、`qpp`、`qpb`、`gop_pic_size` だけを持つ。
- `src/encode.rs` は MinQP / MaxQP、VBV バッファ、Initial VBV Buffer Fullness、QVBR Quality Level、Filler Data、Rate Control Skip Frame、Max AU Size を設定していない。
- プロパティ名と範囲はコーデックごとに異なる。H.264 は `AMF_VIDEO_ENCODER_MIN_QP` / `AMF_VIDEO_ENCODER_MAX_QP`、HEVC は `AMF_VIDEO_ENCODER_HEVC_MIN_QP_I` / `MAX_QP_I` / `MIN_QP_P` / `MAX_QP_P`、AV1 は QIndex (1-255) の intra / inter / inter_B を持つ。
- 実機で H.264 の MinQP / MaxQP / VBVBufferSize / QVBRQualityLevel などの Get / Set が成功した。
- 0020 は既存の公開フィールドが黙って無視される問題を扱う。本 issue は新しい設定項目の追加であり、未対応の組み合わせを黙って無視しない方針を 0020 と揃える。

## 設計方針

- `EncoderConfig` に追加する項目を決める。候補は `min_qp`、`max_qp`、`vbv_buffer_size_bits`、`initial_vbv_buffer_fullness`、`qvbr_quality_level`、`filler_data_enable`、`rate_control_skip_frame_enable`、`max_au_size_bits` とする。
- QP の範囲と単位を検証する。H.264 / HEVC は 0-51、AV1 は 1-255。`qvbr_quality_level` は 1-51、`initial_vbv_buffer_fullness` は 0-64 とする。
- コーデックごとに対応するプロパティがない場合は、無視せず `Err` を返すか doc で明示する。AV1 には `MAX_AU_SIZE` が存在しないため、指定された場合の扱いを決める。
- HEVC の Min / Max QP は I / P でプロパティが分かれる。1 つの値から両方に設定するか、コーデック別の設定型に分けるかを決める。AV1 の QIndex も intra / inter / inter_B の扱いを決める。
- レート制御モードごとに有効な項目を doc に書く。CQP では VBV が無効、QVBR Quality Level は QVBR のときだけ有効、など。
- `ReconfigureParams` に追加するかは、AMF の動的プロパティの可否を確認して決める。初期版は `Init` 時のみでもよいが、その場合は doc に明記する。
- 実機テストを追加する。各コーデックで設定して `Init` とエンコードが成功すること、範囲外の値がエラーになること、未対応の組み合わせがエラーまたは doc どおりの挙動になることを検証する。
- README と `skills/shiguredo-amf/SKILL.md` の対応表を更新する。

## 完了条件

- `EncoderConfig` にレート制御の拡張項目が追加される。
- コーデック別のプロパティ名と値の範囲が実装され、範囲外の値がエラーになる。
- 未対応のコーデックやレート制御モードの組み合わせが、黙って無視されない。
- 有効なレート制御モードと項目の対応が doc に書かれている。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
