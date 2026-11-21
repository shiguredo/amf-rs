# エンコーダーにコーデック別の Min / Max QP 範囲を追加する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-qp-range-params
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

現在の `EncoderConfig` は CQP 用の `qpi` / `qpp` / `qpb` を持つが、レート制御時の QP の下限と上限を指定できない。AMF の既定値に任せるしかなく、低ビットレート時に QP が上がりすぎて画質が落ちるのを抑えたり、QP が下がりすぎて帯域を使いすぎるのを抑えたりできない。AMF はコーデックごとに Min / Max QP のプロパティを持つので、これを設定できるようにする。

## 現状

- `EncoderConfig` に Min / Max QP のフィールドがない。
- `src/encode.rs` の `set_h264_properties`、`set_hevc_properties`、`set_av1_properties` は Min / Max QP を設定していない。
- AMF のプロパティはコーデックごとに異なる。
  - H.264: `AMF_VIDEO_ENCODER_MIN_QP` / `AMF_VIDEO_ENCODER_MAX_QP`。フレーム種別を分けない 1 組で、範囲は 0-51。
  - HEVC: `AMF_VIDEO_ENCODER_HEVC_MIN_QP_I` / `MAX_QP_I` と `MIN_QP_P` / `MAX_QP_P`。I / P で別々の 2 組で、範囲は 0-51。
  - AV1: `AMF_VIDEO_ENCODER_AV1_MIN_Q_INDEX_INTRA` / `MAX_Q_INDEX_INTRA`、`_INTER`、`_INTER_B`。intra / inter / inter_B で別々の 3 組で、範囲は 1-255。
- AMF のヘッダーでは、これらのプロパティはすべて "Dynamic properties" に分類されている。
- `H264EncoderConfig` / `HevcEncoderConfig` / `Av1EncoderConfig` は `profile` だけを持ち、利用箇所は構造体リテラルで構築している。
- 現在のコードは QP の範囲をローカルで検証しない。AMF が返すエラーをそのまま返す。

## 設計方針

- 公開型 `QpRange` を追加する。

```rust
pub struct QpRange {
    /// 下限
    pub min: Option<u16>,
    /// 上限
    pub max: Option<u16>,
}
```

- `min` / `max` はそれぞれ独立に指定でき、`None` の側は AMF の既定値を使う。
- Min / Max QP はコーデック別の設定構造体に置く。共通の `EncoderConfig` フィールドにすると、HEVC の I / P や AV1 の intra / inter / inter_B に同じ値を入れることになり、フレーム種別ごとに違う範囲を指定できなくなるため。
  - `H264EncoderConfig` に `qp_range: Option<QpRange>` を追加し、`AMF_VIDEO_ENCODER_MIN_QP` / `MAX_QP` に設定する。
  - `HevcEncoderConfig` に `qp_range_i` / `qp_range_p: Option<QpRange>` を追加し、`AMF_VIDEO_ENCODER_HEVC_MIN_QP_I` / `MAX_QP_I` と `_P` に設定する。
  - `Av1EncoderConfig` に `qp_range_intra` / `qp_range_inter` / `qp_range_inter_b: Option<QpRange>` を追加し、`AMF_VIDEO_ENCODER_AV1_MIN_Q_INDEX_INTRA` / `MAX_Q_INDEX_INTRA`、`_INTER`、`_INTER_B` に設定する。
- 値の範囲は H.264 / HEVC が 0-51、AV1 が 1-255 であることを doc に書く。ローカルでは検証せず、範囲外の値はそのまま AMF に渡す。`SetProperty` / `Init` が返すエラーはそのまま返す。
- `min` と `max` の大小関係もローカルでは検証しない。AMF の判定に委ねる。
- `qpi` / `qpp` / `qpb` は CQP の固定 QP、`qp_range` はレート制御時の QP の範囲という違いを doc に書く。
- `USAGE` はパラメータ一式を構成するため、`qp_range` は `USAGE` の設定後に設定する。`usage` を指定する機能が入った場合も、明示した `qp_range` が優先されることを確認する。
- 初期版は `EncoderConfig` (Init 時) だけを対象にする。`ReconfigureParams` はフラットな構造体でコーデック別の範囲を表現できないため、本 issue では扱わない。AMF のプロパティは dynamic なので、必要になったらコーデック別の更新型か別メソッドを検討する。
- 構造体リテラルを壊すため、`H264EncoderConfig` / `HevcEncoderConfig` / `Av1EncoderConfig` に `Default` を実装し、`..Default::default()` で移行できるようにする。
- 実機テストを追加する。各コーデックで範囲を指定した `Encoder::new` と数フレームのエンコードが成功すること、HEVC で I / P、AV1 で intra / inter / inter_B に異なる値を指定できることを確認する。設定値の反映は、`src/encode.rs` の単体テストで `PropertyStorage::get_property_int64` を使って確認する。
- README と `skills/shiguredo-amf/SKILL.md` の設定項目を更新する。

## 完了条件

- `QpRange` が公開され、`min` / `max` をそれぞれ指定できる。
- H.264 / HEVC / AV1 のコーデック別設定で Min / Max QP を指定できる。
- H.264 は 1 組、HEVC は I / P、AV1 は intra / inter / inter_B に、指定した値が AMF のプロパティに設定される。
- 範囲外の値をローカルで拒否せず、AMF の判定結果をそのまま返す。
- `qpi` / `qpp` / `qpb` と `qp_range` の違い、および値の範囲が doc に書かれている。
- `EncoderConfig` (Init 時) だけで設定でき、`ReconfigureParams` では変更できないことが doc に書かれている。
- 既存の `H264EncoderConfig` などの構造体リテラルを新しい API に移行し、`..Default::default()` で構築できるようにする。
- README / SKILL.md の設定項目とコード例が実装と一致する。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記する。
