# `AMF_RESOLUTION_CHANGED` を処理し、解像度変化でフレームを欠落・重複させない

- Created: 2026-11-10
- Completed: 2026-11-19
- Branch: feature/fix-resolution-change
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

解像度が変化するストリームをデコードすると、`AMF_RESOLUTION_CHANGED` が返り、現在の `Decoder::decode` はそれをエラーとして扱う。ワーカーは停止せず、以降のフレームを投入しても `AMF_RESOLUTION_CHANGED` が返り続けるため、デコードを継続できない。

解像度変化時の要件は「フレームが欠落せず、同じフレームが 2 回ハンドラーに渡されない」ことである。ライブラリ内で実現できるなら内部で処理し、できないなら利用者がデコーダーを再初期化できるエラーとして返す。

## 現状

- `src/decode.rs` の `Decoder::decode` は、`AMF_RESOLUTION_CHANGED` を非 OK のエラーとして扱う。`drain_output` は `classify_query_output` で解像度変化を `NoOutput` として扱う。
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
- 60 フレームの解像度変化ストリーム (320x240 × 30 + 256x160 × 15 + 320x240 × 15、縮小 → 拡大) を使い、全フレームが 1 回ずつ、サイズごとの枚数が正しいことを検証する。重複は PTS または画素パターンで検出する。
- 変更を起こしたバッファを再投入すると二重デコードになる場合は、内部処理を諦め、`Drain` 後に `Error::ResolutionChanged { width, height, user_data }` を返す。利用者は新しい解像度でデコーダーを作り直し、そのバッファから再開する。
- エラー方式を採る場合は、失敗した入力の `user_data` と新しい解像度を返す API を追加し、再開位置を利用者が特定できるようにする。
- `skills/shiguredo-amf/SKILL.md` と `Decoder::decode` の doc に、解像度変化時の契約を書く。

## 完了条件

- 解像度変化後もデコードが継続し、フレームの欠落と重複がない。
- 内部処理に失敗した場合は、`decode()` が明確なエラーを返す。
- 60 フレームの解像度変化 (縮小 → 拡大) テストが追加される。
- 入力が一度も投入されていない、または最初の入力が受理されていない状態の `finish()` がハングしない。
- `Decoder::decode` の doc と `skills/shiguredo-amf/SKILL.md` に契約が書かれている。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 解決方法

`SubmitInput` が `AMF_RESOLUTION_CHANGED` を返したとき、AMF はそのバッファを受理していない。`Decoder::decode` はワーカーへ `WaitEof` を送る。ワーカーは `Drain` を呼び、`AMF_INPUT_FULL` が返る間は `QueryOutput` を回して出力を通知しながら再試行する。`Drain` を受理したら `AMF_EOF` まで出力を回収し、出力が得られなかった入力をエラーとして通知したあと ack を返す。ワーカーは停止せず、通常ループへ戻る。

`decode()` は `AMF_VIDEO_DECODER_CURRENT_SIZE` から新しい解像度を取得し、`Terminate` と `Init` でコンポーネントを作り直し、受理されなかったバッファを再投入して、同じワーカーへ `Submit` を送って通常ループを再開させる。ハンドラーはワーカーに残るため、呼び出し側との受け渡しは行わない。`finish()` は入力が受理済みなら `WaitEof` でワーカーに Drain と `AMF_EOF` までの回収をさせてから `Stop` で終了し、受理済みの入力がなければ `Stop` だけを送る。最初の `SubmitInput` がハードエラーになって入力を受理していない場合も `Stop` だけを送る。

`WaitEof` で出力が得られなかった入力はエラーとして通知する。`Drain` に失敗した場合、`decode()` はエラーを返し、`finish()` はハンドラーへ通知して `Ok` を返す。`decode()` / `finish()` がエラーを返した後は、`Drop` 以外の操作を保証しない。失敗した場合、変化を検出したフレームはハンドラーへ通知されない。解像度の取得には `amf::PropertyStorage::get_property_size` を追加した。

実機 (AMD Radeon 780M / AMF ランタイム 1.5.3.0) では、AMF は `SubmitInput` で解像度変化を通知し、そのバッファは受理しなかった。`QueryOutput` は解像度変化を通知せず、`Drain` のあと `AMF_EOF` を返した。このため `QueryOutput` が変化を返した場合の再投入は不要で、`SubmitInput` が変化を返した場合だけバッファを再投入している。内部処理だけでフレームの欠落と重複を防げたため、利用者がデコーダーを作り直して再開するためのエラー API は追加していない。`Component::re_init` は `Result.h` が解像度変化に `Drain` / `Terminate` / `Init` を要求しているため使わず、`Terminate` と `Init` を採用した。`ReInit` は試していない。

テストでは、320x240 × 30 + 256x160 × 15 + 320x240 × 15 の 60 フレームを連結したストリーム (縮小 → 拡大) を 1 つのデコーダーでデコードし、60 フレームすべてが 1 回ずつ入力順に通知され、解像度と PTS が入力と一致することを確認している。実装中は HEVC と AV1 でも解像度変化 (縮小) を実機で確認した。

`cargo test --workspace` は doc テストを含む 115 件すべて成功し、`cargo fmt --all -- --check` と `cargo clippy --workspace --all-targets -- -D warnings` も成功した。追加したテストは 3 回連続で成功した。
