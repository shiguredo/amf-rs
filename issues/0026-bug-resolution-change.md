# `AMF_RESOLUTION_CHANGED` を処理し、解像度変化でフレームを欠落・重複させない

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-resolution-change
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

解像度が変化するストリームをデコードすると、`AMF_RESOLUTION_CHANGED` が返り、現在の `Decoder::decode` はそれをエラーとして扱う。ワーカーはそのまま停止するため、以降のフレームをデコードできない。

解像度変化時の要件は「フレームが欠落せず、同じフレームが 2 回ハンドラーに渡されない」ことである。ライブラリ内で実現できるなら内部で処理し、できないなら利用者がデコーダーを再初期化できるエラーとして返す。

## 現状

- `src/decode.rs` の `Decoder::decode` と `drain_output` は、`AMF_RESOLUTION_CHANGED` を非 OK のエラーとして扱う。
- `src/amf.rs` の `Component::re_init` は未使用である。
- 解像度変化を扱うテストがない。
- 兄弟ライブラリの状況は、nvcodec-rs がライブラリ内で destroy + create (任意で reconfigure)、video-toolbox-rs が利用者主導の `update_format`、vpl-rs がエラー返却である。

## 設計方針

- 第一候補は、nvcodec-rs と同じ「ライブラリ内でデコーダーを作り直す」方式とする。`AMF_RESOLUTION_CHANGED` を受けたら、次の順で処理する。
  1. `Drain` して保留中の出力をすべてハンドラーへ渡す
  2. `AMF_VIDEO_DECODER_CURRENT_SIZE` から新しい解像度を取得する
  3. `Terminate` してから `Init(new_width, new_height)` で作り直す
  4. `SubmitInput` が変更を返した場合は、そのバッファを新しいデコーダーへ再投入する
- `QueryOutput` が変更を返した場合と `SubmitInput` が変更を返した場合で、再投入の要否が異なる。実機で確認して分岐する。
- 45 フレームの解像度変化ストリーム (320x240 × 30 + 256x160 × 15) を使い、全フレームが 1 回ずつ、サイズごとの枚数が正しいことを検証する。重複は PTS または画素パターンで検出する。
- 変更を起こしたバッファを再投入すると二重デコードになる場合は、内部処理を諦め、`Drain` 後に `Error::ResolutionChanged { width, height, user_data }` を返す。利用者は新しい解像度でデコーダーを作り直し、そのバッファから再開する。
- エラー方式を採る場合は、失敗した入力の `user_data` と新しい解像度を返す API を追加し、再開位置を利用者が特定できるようにする。
- `skills/shiguredo-amf/SKILL.md` と `Decoder::decode` の doc に、解像度変化時の契約を書く。

## 完了条件

- 解像度変化後もデコードが継続し、フレームの欠落と重複がない。
- 内部処理が不可能な場合は、利用者が再開できる明確なエラーが返る。
- 45 フレームの解像度変化テストが追加される。
- `Decoder::decode` の doc と `skills/shiguredo-amf/SKILL.md` に契約が書かれている。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。
