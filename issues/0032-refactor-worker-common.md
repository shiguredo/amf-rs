# encode / decode のワーカー実装を共通化し、PTS 対応付けに統一する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-worker-common
- Polished: {YYYY-MM-DD}
- Refreshed: 2026-11-15
- Reporter: @melpon

## 目的

`src/encode.rs` と `src/decode.rs` のワーカー実装がほぼコピペになっている。`WorkerCommand`、ワーカーのループ、`QueryOutput` のポーリング、`finish`、`Drop`、`SubmitInput` のリトライが重複しており、片方だけ修正されて不整合になるリスクがある。

また、出力と `user_data` の対応付けがエンコーダーは FIFO、デコーダーは PTS と分かれている。エンコーダーも PTS で対応付けできれば、worker 全体を 1 つの実装にでき、将来 B フレームに対応するときも対応付けを作り直さずに済む。

## 現状

- `WorkerCommand`、ワーカーのループ、`finish`、`Drop`、`SubmitInput` のリトライが encode / decode で重複している。
- この issue は 0013 の変更を前提とする。0013 の対応で `QueryOutput` の結果分類は `src/query_output.rs` に共通化され、終端エラーの通知方法も両方で同じ形になる。ただし、その後の出力処理と終了処理はまだ別々に実装されている。
- エンコーダーは `VecDeque` の FIFO で出力と `user_data` を対応付けており、出力順が入力順と一致することを前提にしている。
- デコーダーは `HashMap<amf_pts, PendingFrame>` で PTS をキーに対応付けている。
- エンコーダーは `CodecConfig` を丸ごと clone してワーカーへ渡している。
- 実機計測 (H.264 / HEVC / AV1、320x240 NV12、5 フレーム、フレームレート 7/3) では、エンコーダーの出力 `Buffer::get_pts()` が入力 `Surface` の PTS と一致した。
  - 例: 入力 PTS `4285714` に対して出力 Buffer PTS も `4285714`
  - 3 コーデックとも出力フレーム数は入力と同じ 5 で、順序も一致した
- エンコーダーに `AMF_TIMESTAMP_MODE` に相当するプロパティはない。`AMF_VIDEO_ENCODER_PRESENTATION_TIME_STAMP` はハードウェアエンコーダーでは `AMF_NOT_FOUND` になり、`Buffer::get_pts()` を使う必要がある。
- 上記の計測は `Encoder::encode()` がフレームレートから計算して設定した PTS を使っている。任意の不規則な PTS を設定した場合のパススルーはまだ確認していない。
- 現行の FIFO も 1 入力 1 出力を前提にしているため、複数出力や出力順の変化には対応できない。

## 設計方針

- 共通のワーカーモジュールを作り、コマンド処理、`QueryOutput` のポーリング、出力の対応付け、終了処理、`Drop` を 1 箇所にまとめる。
- トレイトは作らず、enum とクロージャで差分を渡す。
  - 入力と出力の型、出力の変換、ハンドラーへの通知を enum / クロージャで表現する
  - 対応付けの違いは `Matcher` のような enum にせず、PTS の `HashMap<amf_pts, PendingFrame<U>>` に統一する
- エンコーダーもデコーダーと同じ PTS 対応付けにする。
  - `Submit` は `user_data` と対応付け用 PTS、元の PTS を持つ
  - `Output` は出力種別と PTS を持つ
  - `Converter` のエンコーダー側は `Buffer::get_pts()` を読む
  - `Encoder::encode()` は入力 `Surface` に一意な PTS を設定し、呼び出し元が使う提示時刻は別に保持する
  - 通知前に出力 `Buffer::set_pts()` で元の提示時刻へ戻す
- 出力の変換エラーは、デコーダーと同じ条件で扱う。
  - `Err` は対応付け用 PTS を持たないため、`pending` を消費せずにハンドラーへ通知する
  - 対応する入力は `finish` 時に「出力が得られなかった入力」として通知する
- 同じ PTS の出力が複数ある場合は、`HashMap<amf_pts, VecDeque<PendingFrame<U>>>` にするか、複数出力を許さないかを実測して決める。
- 実装前に、任意の不規則な PTS を入力 `Surface` に設定して、出力 `Buffer::get_pts()` に反映されることを確認する。反映されない場合はエンコーダーだけ FIFO を残し、worker の共通化のみを行う。
  - 公開 API では `Encoder::encode()` が PTS を設定するため、任意 PTS の確認は一時的なテスト経路で行い、確認後は削除する
- `CodecConfig` は `Arc` で共有し、ワーカー用に全体を clone しない。
- 既存の公開 API は変えない。通常系の挙動も変えないが、変換エラー時の通知はデコーダー方式に揃えるため、未処理入力のエラーが追加で通知される場合がある。
- `SubmitInput` のリトライ方針を共通化し、`AMF_REPEAT` / `AMF_NEED_MORE_INPUT` の扱いをエンコーダーとデコーダーで切り替える。
- 作業量が大きいため、フェーズを分ける。
  1. 任意 PTS のパススルー確認
  2. worker の共通化
  3. PTS 対応付けへの統一
  4. 不要になった `Matcher` の削除とテスト

## 完了条件

- ワーカーの共通部分が 1 箇所にまとまっている。
- encode / decode の差分が、入力の構築、出力の変換、ハンドラーへの通知だけになっている。
- エンコーダーとデコーダーが同じ PTS 対応付けを使っている。
- エンコーダーの出力 `Buffer` の PTS が、通知前に元の提示時刻へ戻っている。
- 変換エラーが `pending` を消費せず、デコーダーと同じ条件で通知される。
- 任意の PTS でのパススルーが確認され、確認できない場合は FIFO を残す判断が issue に記録されている。
- ワーカーが不要に `CodecConfig` 全体を複製していない。
- 既存のテストがすべて通る。
- `cargo clippy --workspace --all-targets -- -D warnings` が通る。
- `CHANGES.md` の `## develop` の `### misc` にリファクタリングとして追記する。
