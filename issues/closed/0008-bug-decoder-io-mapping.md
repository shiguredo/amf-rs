# Decoder の `user_data` を PTS ベースで対応付ける

- Created: 2026-11-10
- Completed: 2026-11-14
- Branch: feature/fix-decoder-io-mapping
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Decoder::decode(buffer, user_data)` の `user_data` が、B フレームを含むストリームで誤ったフレームに紐付く。AMF API Reference は「User code should not make any assumptions about any relationship between input and output samples.」と明記しており、デコーダーの既定の `AMF_VIDEO_DECODER_REORDER_MODE` は `AMF_VIDEO_DECODER_MODE_REGULAR` (DPB による並べ替えあり) であるため、出力順は入力順と一致しない。

`skills/shiguredo-amf/SKILL.md` は `user_data` が `encode()` / `decode()` に渡したフレームと 1 対 1 で対応すると保証しているが、実装は保証していない。

## 現状

- `src/decode.rs` の `worker` / `drain_output` が `pending` (user_data) と `output_buffer` を FIFO で 1 対 1 に対応付けている。
- `Decoder::decode` は入力 `Buffer` に PTS を設定しない。`src/amf.rs` の `Buffer` に `set_pts` がなく、`Surface` に `get_pts` がない。
- `AMF_TIMESTAMP_MODE` を設定していない (既定は `AMF_TS_PRESENTATION`)。
- B フレーム入りストリームのテストがない。`tests/test_roundtrip.rs` の `decode` ヘルパーは `()` を渡すだけで `user_data` を検証していない。

## 設計方針

- `src/amf.rs` の `Buffer` に `set_pts`、`Surface` に `get_pts` を追加する。
- `Decoder::decode` で入力 `Buffer` に単調増加の一意 ID を PTS として設定する。フレームレート由来の時刻ではなく、入出力の対応付け用の ID とする。
- `AMF_TIMESTAMP_MODE` を `AMF_TS_DECODE` または `AMF_TS_SORT` に設定する。`AMF_TS_PRESENTATION` の意味が AMF のヘッダーと API リファレンスで異なるため、どちらが実機で入力 PTS を保持するかを確認してから確定する。
- `worker` の `pending` を `HashMap<amf_pts, VecDeque<UserData>>` などに変え、出力 `Surface` の PTS で `user_data` を引き当てる。
- PTS が取れない、または一致しない出力はエラーとして通知し、FIFO で誤魔化さない。
- B フレーム入り H.264/HEVC のテストを追加し、`user_data` と出力フレームの対応を検証する。
- 複数フレームを含むバッファ (`AMF_REPEAT`) の扱いは別 issue とする。本 issue は 1 バッファ 1 フレームを前提とする。
- エンコーダー側も FIFO 前提を持つが、現在のエンコーダーは B フレームを生成しないため実害がない。エンコーダー側の対応は別 issue とし、本 issue のスコープ外とする。

## 完了条件

- B フレーム入りストリームで `user_data` が正しいフレームに付く。
- 対応するテストが追加され、既存のラウンドトリップテストが通る。
- `Decoder::decode` の doc と `skills/shiguredo-amf/SKILL.md` の 1 対 1 の記述を実装に合わせる。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 解決方法

- `amf::Buffer` に `set_pts`、`amf::Surface` に `get_pts` を追加した。
- `Decoder::new` で `AMF_TIMESTAMP_MODE` を `AMF_TS_PRESENTATION` に設定するようにした。
  - 実機の H.264/HEVC デコーダーでは `AMF_TS_PRESENTATION` が入力 PTS を出力 Surface へ引き継ぐ。
  - `AMF_TS_DECODE` と `AMF_TS_SORT` は出力順にタイムスタンプを振り直すため、対応付けには使えないことを実機で確認した。
- `Decoder::decode` で入力 Buffer に 1 から始まる一意な PTS を割り当てるようにした。
  - 呼び出し元が入力 Buffer に設定した PTS は保存し、対応する出力 Surface を通知する前に元の値へ戻す。
- ワーカーの `pending` を PTS をキーにした `HashMap` に変更し、出力 Surface の PTS で `user_data` を引き当てるようにした。
  - `Submit` コマンドの処理が入力の投入より遅れた場合に備え、対応する `user_data` が登録されるまで出力を保持する。
  - 対応する入力がない出力と、出力が得られなかった入力はハンドラーにエラーとして通知する。
  - `finish` 後は入力フレームを受け付けず、`decode` を呼ぶとエラーを返す。
- `finish` はワーカーの完了まで待つようにした。
- B フレームを含む H.264/HEVC のビットストリームをテストデータとして追加し、`user_data` と PTS の対応、および出力順が入力順と異なることを検証するテストを追加した。
- `Decoder::decode` の doc、`README.md`、`skills/shiguredo-amf/SKILL.md` を実装に合わせて更新した。
- AMD GPU 実機で `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace -- --test-threads=1` を通した。
