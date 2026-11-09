# エンコーダー / デコーダーのケーパビリティ照会 API を追加する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-capability-query
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`supported_codecs()` は固定のプロファイル一覧と固定のフラグだけを返すため、GPU ごとの解像度範囲、対応フォーマット、メモリタイプ、最大ビットレート、B フレーム対応などを利用者が知ることができない。vpl-rs、nvcodec-rs、video-toolbox-rs はケーパビリティ照会 API を持っており、設定値の事前検証、UI、コーデック選択に使える。amf-rs にも同等の API が必要である。

## 現状

- `src/codec_info.rs` の `supported_codecs` は `CreateComponent` の成否と固定のプロファイル一覧だけを返す。
- `EncodingInfo` の `profiles`、`supports_frame_reordering`、`supports_multi_pass`、`hardware_accelerated` は実機の値ではなく固定値である。
- `src/amf.rs` の `ComponentFuncs` は `GetCaps` を保持しておらず、`AMFCaps` / `AMFIOCaps` のラッパーもない。
- `AMFComponent::GetCaps` は実機の H.264 エンコーダーで成功し、最大ビットレート 1 Gbps、最大プロファイル High、最大レベル 5.2、B フレーム / ROI / pre-analysis 対応、入力 128..4096、6 フォーマット、HOST / OpenCL / OpenGL を返した。デコーダー側の値は未取得である。
- `issues/0022-bug-codec-info.md` は `supported_codecs` の内容を実機に合わせるバグ修正である。本 issue は利用者が任意のコーデックについて詳細な能力を照会できる公開 API の追加であり、目的と完了条件が異なる。`GetCaps` の内部実装は 0022 と共通化する。

## 設計方針

- `src/amf.rs` の `Component` に `get_caps()` を追加し、`Caps` / `IoCaps` の RAII ラッパーを追加する。`Acquire` / `Release` の管理は既存のラッパーと同じ方針にする。
- 公開 API は `query_encoder_caps(codec: VideoCodecType) -> Result<EncoderCaps, Error>` と `query_decoder_caps(codec: VideoCodecType) -> Result<DecoderCaps, Error>` を基本とする。`supported_codecs` はこの結果を使って実機の値に基づく情報を返す。
- `IoCaps` は幅・高さの範囲、対応するサーフェスフォーマット、対応するメモリタイプ、インターレース対応を持つ。`ffi` の型を公開せず、公開用の enum に写像する (0023 の方針と整合させる)。
- `EncoderCaps` は最大ビットレート、最大プロファイル、最大レベル、B フレーム対応、ROI 対応、pre-analysis 対応、最大テンポラルレイヤー数、HW インスタンス数、色変換のアクセラレーション種別を持つ。
- `DecoderCaps` は最大ストリーム数、HW インスタンス数、入出力の `IoCaps` を持つ。
- `GetCaps` を呼ぶにはコンポーネントの `Init` が必要である。一時的な `Context` と `Component` を作り、最小サイズ (例: 128x128) で `Init` して `GetCaps` し、すぐ `Terminate` する。GPU リソースの消費と、`Init` に失敗した場合のエラーを doc に書く。
- `GetCaps` が返すフォーマット一覧は、実際に `Init` できるフォーマットと一致しない場合がある。実機では YUY2 が入力フォーマット一覧にないが `Init` は成功した。caps は「AMF が報告する能力」であり、初期化やエンコードの成功を保証しないことを doc に書く。
- `GetCaps` の取得結果をキャッシュするか毎回問い合わせるかを決める。AMF の能力はプロセス内で変わらないため、キャッシュを検討する。
- 実機テストを追加する。各コーデックで caps が取得できること、幅・高さの範囲が正の値であること、`supported_codecs` の値と矛盾しないこと、未知の値でもパニックしないことを検証する。

## 完了条件

- `query_encoder_caps` / `query_decoder_caps` が公開され、`EncoderCaps` / `DecoderCaps` / `IoCaps` を返す。
- `supported_codecs` がケーパビリティ照会の結果を使い、固定値ではない値を返す。
- `ffi` 型が公開 API に現れない。
- `GetCaps` の値が初期化成功を保証しないことが doc に書かれている。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
