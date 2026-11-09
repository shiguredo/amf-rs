# `codec_info` の能力情報を実機と一致させる

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-codec-info
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`supported_codecs` が返す `CodecInfo` の能力情報が、実機の AMF と無関係の固定値になっている。利用者はプロファイルや B フレーム対応を正しいものと信じて使うため、誤った情報を返さないようにする。

## 現状

- `EncodingInfo` の `profiles` はヘッダー定義をもとにした固定の一覧で、ハードウェアが未対応のプロファイルも含み得る。
- `supports_frame_reordering` は H.264 / HEVC が常に true、AV1 が常に false である。`supports_multi_pass` は常に false である。
- `hardware_accelerated` は常に `supported` と同じ値で、情報量がない。
- コーデックの対応判定は `CreateComponent` の成否だけで、`Init` を試していない。
- 「AMF SDK にはプロファイル一覧を問い合わせる API がない」というコメントがあるが、`AMFComponent::GetCaps` と `AMFCaps` に `AMF_VIDEO_ENCODER_CAP_MAX_PROFILE`、`AMF_VIDEO_ENCODER_CAP_BFRAMES` などの能力照会がある。
- `ProbeContext` は `Context` の薄いラッパーで、`Drop` で `terminate` する。

## 設計方針

- `AMFComponent::GetCaps` をバインドし、`AMFCaps` から最大プロファイル、最大レベル、B フレーム対応などを取得する。
- プロファイルの一覧を列挙する API がない場合は、`CAP_MAX_PROFILE` を上限として、ヘッダー定義のうち実際に使えるものを返す。固定値であることと根拠を doc に書く。
- `hardware_accelerated` は公開 API から削除するか、実装がハードウェアのみであることを理由に常に true として doc に明記する。`supported` との重複を避ける。
- エンコーダーの対応判定は `CreateComponent` だけでなく `Init` まで試す。デコーダーも同様に、可能なら `Init` を試す。
- 実機で得た `AMFCaps` の値と `codec_info` の出力を比較するテストを追加する。
- `AMFCaps` を使わない場合でも、固定値であることとハードウェア依存であることを doc と README に明記する。

## 完了条件

- `CodecInfo` の能力情報が、実機の `AMFCaps` または明示した固定値のどちらかに基づいて説明できる。
- 誤った「照会 API がない」というコメントが削除される。
- `hardware_accelerated` の扱いが決まり、doc に書かれている。
- 全 false の `EncodingInfo` の手書き重複を `not_supported()` にまとめる。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に変更を追記する。
