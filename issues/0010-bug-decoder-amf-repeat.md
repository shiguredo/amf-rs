# Decoder の `AMF_REPEAT` 処理を修正し、複数フレームを含むバッファをデコードできるようにする

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-decoder-amf-repeat
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

AMF の仕様では、`AMFComponent::SubmitInput` が `AMF_REPEAT` を返した場合、現在のバッファに複数のフレームが含まれているため、`NULL` を引数に `SubmitInput` を呼んで残りのデータを処理する必要がある。

現在の `Decoder::decode` は `AMF_REPEAT` をリトライとして扱い、同じバッファを再投入して 100 回で打ち切る。そのため、複数フレームを含むバッファはデコードできない。実機では 10 フレーム分のビットストリームを連結した 1 バッファの投入で `Decoder::decode(): SubmitInput retry limit exceeded` になり、出力は 0 件になった。

## 現状

- `src/decode.rs` の `Decoder::decode` は、`AMF_INPUT_FULL` / `AMF_DECODER_NO_FREE_SURFACES` / `AMF_REPEAT` を同じリトライ経路で扱い、同じ `buffer.as_ptr()` を再投入する。
- `NULL` を引数に `SubmitInput` を呼ぶ経路が存在しない。
- テストは 1 バッファ 1 フレームだけを投入しており、複数フレームを含むバッファを検証していない。
- `AMF_Video_Decode_API` には、`AMF_REPEAT` の後は `NULL` を渡して残りを処理する旨が明記されている。

## 設計方針

- `AMF_REPEAT` を受けたら、`SubmitInput(NULL)` を `AMF_OK` または `AMF_NEED_MORE_INPUT` まで呼び、残りのフレームを処理する。
- 1 つの入力バッファから複数の出力が得られるため、`user_data` の割り当てを決めて doc に書く。1 バッファに 1 つの `user_data` を割り当て、最初の出力にだけ渡す方針を基本とする。
- `NULL` 投入のループにも上限を設け、無限ループを防ぐ。上限に達した場合はエラーとして通知する。
- 1 フレームが複数のバッファに分割されて入力されるケースは、入力順のままで問題ない。現在の `decode()` の doc が 1 バッファ 1 フレームを前提としている場合は修正する。
- 複数フレームを含むバッファのテストを追加する。AMF のエンコーダーで生成した複数フレームのビットストリームを連結して 1 バッファにする方法でよい。
- PTS ベースの `user_data` 対応付け (別 issue) と整合する形にする。

## 完了条件

- 複数フレームを含む 1 バッファを投入しても、すべてのフレームがデコードされる。
- `AMF_REPEAT` の処理を検証するテストが追加される。
- `Decoder::decode` の doc が実装に合っている。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。
