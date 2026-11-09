# Decoder の `user_data` を PTS ベースで対応付ける

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
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
