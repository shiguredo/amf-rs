# encode / decode のワーカー実装を共通化し、frame ID で入出力を対応付ける

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-worker-common
- Polished: {YYYY-MM-DD}
- Refreshed: 2026-11-20
- Reporter: @melpon

## 目的

`src/encode.rs` と `src/decode.rs` のワーカー実装がほぼコピペになっている。`WorkerCommand`、ワーカーのループ、`QueryOutput` のポーリング、出力の配信、`Drop`、`SubmitInput` のリトライが重複しており、片方だけ修正されて不整合になるリスクがある。

また、出力と `user_data` の対応付けが、エンコーダーは frame ID、デコーダーは内部 PTS と分かれていた。デコーダーもエンコーダーの frame ID 方式に統一すれば、PTS を識別子として上書きする必要がなくなり、ワーカー全体を 1 つの実装にできる。

## 現状

- `WorkerCommand`、ワーカーのループ、`drain_output`、`deliver_buffered`、`flush_buffered_outputs`、`report_unmatched_inputs`、`notify_terminal_error`、`Drop` が encode / decode で重複していた。
- 終了処理は方式が分かれていた。エンコーダーは呼び出し元の `Encoder::finish` が `AMFComponent::Drain` を呼び、`WorkerCommand::Finish` でワーカーに終了を指示していた。デコーダーは `WorkerCommand::WaitEof` でワーカーに Drain と `AMF_EOF` までの回収を任せ、`WorkerCommand::Stop` で終了していた。デコーダーは `drain_required` と `initialized` を持ち、解像度変化時の再初期化と同期していた。
- 0013 で `QueryOutput` の結果分類 (`src/query_output.rs` の `classify_query_output`) と終端エラー通知が共通化済みである。
- 0024 で `finish` 後の操作拒否、`finish` の join 待ち、`Drop` の join と `AMFComponent::Terminate` の扱いが確定した。`Encoder::finish` は Drain に失敗した場合に `Err` を返す。0032 でエラー後の再試行保証は廃止し、呼び出し側に保証するのは `Drop` だけとした。
- 0025 でエンコーダーの対応付けが frame ID 方式に変わった。`Encoder::encode` が入力 `Surface` の `ShiguredoAmfFrameId` プロパティに一意な `i64` を設定し、0032 の共通ワーカーでは `drain_output` が出力 Buffer から同じプロパティを読む。B フレームでは AMF が付ける PTS をフレームの識別に使えないことを実機で確認している。エンコーダーは入力 PTS を変更せず、出力 `Buffer` の PTS も設定しない。
- 変更前のデコーダーは PTS 方式だった。`Decoder::decode` が入力 `Buffer` の元 PTS を保存し、`next_pts` の連番を `Buffer::set_pts` で設定し、出力 `Surface` の PTS をキーに `HashMap<amf_pts, PendingFrame>` から `user_data` を引く。通知前に `Surface::set_pts` で元の PTS へ戻す。`set_timestamp_mode` が `AMF_TIMESTAMP_MODE` に `AMF_TS_PRESENTATION` を設定している。
- デコーダーの入力 PTS を出力 `Surface` へ引き継ぐ挙動は `README.md` と `skills/shiguredo-amf/SKILL.md` に記載され、`tests/test_decode.rs` でも検証していた。今回の変更でこの公開契約を廃止した。
- デコーダーの入力 `Buffer` に設定した `ShiguredoAmfFrameId` が出力 `Surface` へ引き継がれることを、H.264 / HEVC / AV1 の実機で確認した。エンコーダーの `Surface` から `Buffer` への引き継ぎとは経路が異なる。
- `Encoder::new` は `CodecConfig` を 1 回 clone してワーカーへ渡している。フレームごとの clone はない。
- 0026 でデコーダーの解像度変化対応が入り、`WorkerCommand::WaitEof` / `Stop` 方式と `drain_required`、再初期化の同期が確立した。
- 0028 はワーカーの出力回収が入力処理を starvation させる問題を扱う。0032 の共通ワーカーではコマンド処理後の出力回収を実装したが、`SubmitInput` のリトライ中の出力回収は未実装で、0028 の完了条件として残っている。

## 設計方針

- 共通のワーカーモジュールを作り、コマンド処理、`QueryOutput` のポーリング、出力の対応付け、配信、終了処理、`Drop` を 1 箇所にまとめる。
- トレイトは作らず、enum とクロージャで差分を渡す。
  - ハンドラーへの通知と終了条件、解像度変化時の再初期化フックを差分として渡す
  - ワーカーの制御はデコーダー方式 (`RegisterUserData` / `WaitEof` / `Stop`) を土台にする
  - `QueryOutput` が返す `AMFData` は `amf::Data` で包み、frame ID の読み取りと出力の保持を共通ワーカーで行う
  - ワーカーが `AMFComponent::Drain` と `AMF_EOF` までの回収を行う。呼び出し側は `drain_required` を管理し、`finish` で `WaitEof` の後に `Stop` を送る
  - `Encoder::finish` / `Decoder::finish` は同じ処理にまとめる。`finish` がエラーを返した場合、呼び出し側に保証するのは `Drop` だけとする
  - `Drop` は `Stop` コマンドを送り、キューにあるコマンドを処理してから join する。drop の処理中に通知される入出力があるため、すべてのフレームの通知は保証しない
  - 解像度変化時の再初期化は `Decoder` 固有のフックとして残し、`WaitEof` の完了でワーカーと同期する
- 入出力の対応付けは、frame ID の `HashMap<i64, U>` に統一する。
  - `RegisterUserData` は frame ID と `user_data` を持つ。PTS は持たない
  - frame ID の発行、入力プロパティへの設定、`SubmitInput` の呼び出しは共通ワーカーモジュールで行う
  - `QueryOutput` の出力は `Data` と frame ID の組で保持する
  - エンコーダーは入力 `Surface` の `ShiguredoAmfFrameId` に設定し、出力 `Buffer` から読む
  - デコーダーは入力 `Buffer` の `ShiguredoAmfFrameId` に設定し、出力 `Surface` から `Surface::convert` の前に読む
  - プロパティ名 `ShiguredoAmfFrameId` はライブラリの予約名として `README.md` と `skills/shiguredo-amf/SKILL.md` に明記する
- デコーダーもエンコーダーと同じく PTS をライブラリ側で扱わない。
  - `Decoder::decode` は入力 `Buffer` の PTS を変更しない
  - 出力 `Surface` の PTS も設定しない。AMF が付けた値のままにする
  - `next_pts`、`original_pts`、`PendingFrame::original_pts`、`Surface::set_pts` による復元、`set_timestamp_mode` を削除した
  - 入力 PTS の重複・非単調、B フレーム、解像度変化の各ケースで frame ID の対応付けが成立することを実機で確認した
  - PTS を出力へ引き継ぐ契約を廃止するため、`README.md`、`skills/shiguredo-amf/SKILL.md`、`tests/test_decode.rs`、`CHANGES.md` を更新した。`CHANGES.md` には `[CHANGE]` を追記した
- `Data` の作成や frame ID の読み取りに失敗した場合は対象入力を特定できないため、未対応出力として通知し、その入力は後で未対応入力として通知する。`Buffer` / `Surface` への変換エラーは `pending` を消費してから通知する。
- `Encoder::finish` の Drain 失敗時は `Err` を返す。エラー後の再試行は保証せず、呼び出し側に保証するのは `Drop` だけとする。
  - ワーカーの終了条件は、デコーダーが `AMF_EOF` まで回収し、エンコーダーが `AMF_EOF` または保留中の入出力が空になるまでとする。この差分は終了条件のフックで渡す
  - エンコーダーの Drain が `AMF_INPUT_FULL` を返した場合は出力を回収しながら再試行する。Drain 後に `AMF_EOF` が返ることは実機で確認した
- `SubmitInput` のリトライを共通化した。共通のリトライ対象は `AMF_INPUT_FULL` と `AMF_DECODER_NO_FREE_SURFACES` とし、それ以外の status はそのまま呼び出し側へ返す。
  - エンコーダー: `AMF_OK` と `AMF_NEED_MORE_INPUT` を受理する。`AMF_REPEAT` は AMF のエンコーダー API で定義されていないため、エラーとして扱う
  - デコーダー: `AMF_OK`、`AMF_NEED_MORE_INPUT`、`AMF_RESOLUTION_CHANGED` を受理する。`AMF_REPEAT` は複数フレームを含むバッファとしてエラーにし、残りの出力を回収するために Drain を必要とする状態にする
  - `AMF_DECODER_NO_FREE_SURFACES` はデコーダー専用の status だが、一時的なリソース枯渇を表すため共通のリトライ対象に含める。エンコーダーはこの status を返さない
- 0028 の starvation 対策として、コマンドを 1 つ処理した後にも `drain_output` を呼ぶ。`SubmitInput` のリトライ中の `QueryOutput` は未実装で、0028 の完了条件として残っている。
- 共通ワーカーの出力型と再初期化フックは固定しない。0042 の出力フォーマットと 0046 のデコーダープロパティを後から追加できる形にする。再初期化時は `DecoderConfig` 由来の設定を再適用する。
- `CodecConfig` の `Arc` 化は行わない。構築時に 1 回 clone するだけでフレームごとのコピーはなく、AGENTS.md の「Premature Optimization is the Root of All Evil」に照らして現時点で必要ない。
- 1 入力 1 出力の前提は維持する。対応する入力がない出力は `Err` として通知する。
- 既存の `Encoder` / `Decoder` の公開シグネチャは変えない。`amf::Data` と `Surface::data` / `Buffer::data` / `into_data` を追加し、デコーダーの出力 PTS と `finish` / `Drop` の挙動が変わるため、`CHANGES.md` に `[ADD]` / `[CHANGE]` を追記する。
- 作業量が大きいため、次のフェーズに分けて進め、0028 のリトライ中の出力回収を除いて完了した。
  1. デコーダー入力 `Buffer` の `ShiguredoAmfFrameId` が出力 `Surface` へ引き継がれることの実機確認
  2. 共通ワーカーモジュールの抽出 (デコーダー方式の制御、ワーカー側 Drain、frame ID 対応付け)
  3. エンコーダーを共通ワーカーへ移行し、`Finish` 方式を廃止する
  4. デコーダーを frame ID 対応付けへ移行し、PTS 関連のコードを削除する
  5. テスト、`README.md`、`skills/shiguredo-amf/SKILL.md`、`CHANGES.md` の更新

## 完了条件

- ワーカーの共通部分が 1 箇所にまとまっている。
- encode / decode の差分が、deliver での出力変換、ハンドラーへの通知、終了条件、再初期化フックに閉じている。
- エンコーダーとデコーダーが同じ `ShiguredoAmfFrameId` 対応付けを使っている。
- デコーダーが入力 `Buffer` の PTS を変更せず、出力 `Surface` の PTS も設定していない。
- デコーダー入力 `Buffer` の frame ID が出力 `Surface` へ引き継がれることを、H.264 / HEVC / AV1 の実機で確認している。引き継がれない場合は frame ID 統一を中止し、PTS の扱いを含む今後の方針をユーザーと決めて issue に記録している。
- `Data` の作成や frame ID の読み取りに失敗した場合は未対応出力として通知し、その入力は後で未対応入力として通知される。`Buffer` / `Surface` への変換エラーは `pending` を消費してから通知される。
- `finish` がエラーを返した場合、`Drop` 以外の操作を保証しない。`finish` 後の操作は拒否され、`Drop` はキューにあるコマンドを処理してからワーカーを join し、`AMFComponent::Terminate` を呼ぶ。drop の処理中に通知される入出力があるため、すべてのフレームの通知は保証しない。
- `Encoder::finish` / `Decoder::finish` は同じ処理を使い、エラー後の `Drop` 以外を保証しない契約が同じである。
- デコーダーの PTS 契約の変更に合わせて、`README.md`、`skills/shiguredo-amf/SKILL.md`、`tests/test_decode.rs` が更新されている。
- `ShiguredoAmfFrameId` がライブラリの予約プロパティ名であることが `README.md` と `skills/shiguredo-amf/SKILL.md` に明記されている。
- `amf::Data` と `Surface::data` / `Buffer::data` / `into_data` が追加され、`to_buffer` / `to_surface` で参照カウントを増やした `Buffer` / `Surface` を取得できる。
- `Drop` の契約変更に合わせて、エンコーダーの「Drop で保留入力を通知しない」テストを削除している。
- 0028 のポーリング方針のうち、コマンド処理後の出力回収は共通ワーカーのループに反映されている。`SubmitInput` のリトライ中の出力回収は未実装で、0028 の完了条件として残っている。
- 0042 / 0046 が後から出力型とデコーダー設定を追加できるフックになっている。
- `SubmitInput` のリトライ対象が encode / decode で共通化され、エンコーダーの `AMF_REPEAT` はリトライされない。
- 既存のテストがすべて通る。
- `cargo clippy --workspace --all-targets -- -D warnings` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` (`amf::Data`) と `[CHANGE]` (デコーダーの PTS、`finish`、`Drop`) を追記し、`### misc` にリファクタリングとして追記した。既存の `[FIX]` にあるデコーダーの PTS 対応付けと `Drop` の記述も、今回の実装に合わせて更新した。
