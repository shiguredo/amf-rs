# B フレームを有効にしたエンコーダーの `AMF_NEED_MORE_INPUT` と出力順の対応付けを修正する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-bframe-support
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

B フレームを有効にすると、AMF のエンコーダーは最初の出力を出すまでに複数の入力サンプルを必要とし、`SubmitInput` が `AMF_NEED_MORE_INPUT` を返す。現在の `Encoder::encode` はこれをエラーとして扱うため、B フレームを有効にできない。

また、現在のワーカーは `user_data` と出力を FIFO で 1 対 1 に対応付けている。B フレームでは入力順と出力順が一致しないため、`user_data` が誤ったフレームに付く。AMF の仕様も入出力の対応を仮定することを禁じている。

## 現状

- `EncoderConfig` に B フレームの設定がなく、`AMF_VIDEO_ENCODER_B_PIC_PATTERN` を設定していない。
- `Encoder::encode` の `SubmitInput` リトライループは、`AMF_OK` / `AMF_INPUT_FULL` / `AMF_REPEAT` 以外をエラーにしている。`AMF_NEED_MORE_INPUT` はここでエラーになる。
- `src/encode.rs` のワーカーは `pending` (user_data) と `output_buffer` を FIFO で対応付けている。
- 入力の PTS は `frame_count` から計算しており、B フレームの出力順を識別する情報としては使っていない。
- デコーダー側の B フレーム対応は別 issue で扱う。

## 設計方針

- `EncoderConfig` に B フレームの枚数またはパターンを追加し、`AMF_VIDEO_ENCODER_B_PIC_PATTERN` を設定する。HEVC / AV1 の対応するプロパティも確認する。
- `AMF_NEED_MORE_INPUT` は「入力を受け付けたが、まだ出力がない」として扱い、エラーにしない。入力の受付が完了したものとして次のフレームへ進む。
- 入力 `Surface` に一意な PTS を設定し、出力 `Buffer` の PTS で `user_data` を引き当てる。FIFO の 1 対 1 前提をやめる。
- 複数の入力が 1 つの出力に対応する場合と、1 つの入力が複数の出力に対応する場合の扱いを決め、doc に書く。
- B フレームの PTS / DTS の扱いを doc に書く。出力 `Buffer` の PTS が提示順であることを前提にする。
- H.264 / HEVC の B フレーム入りストリームで、全フレームが 1 回ずつ出力され、`user_data` が正しいフレームに付くことをテストする。
- デコーダー側の PTS 対応付けと整合させる。

## 完了条件

- B フレームを有効にしたエンコーダーが `AMF_NEED_MORE_INPUT` で停止しない。
- 入力順と出力順が異なっても、`user_data` が正しいフレームに付く。
- 全フレームが 1 回ずつ出力される (欠落・重複がない)。
- B フレーム入り H.264 / HEVC のテストが追加される。
- `EncoderConfig` と `encode()` の doc が実装と一致する。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
