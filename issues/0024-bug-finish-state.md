# `Encoder` / `Decoder` の `finish` と `Drop` の状態を明確にする

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-finish-state
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`finish()` の後に `encode()` / `decode()` を呼ぶと、AMF への `SubmitInput` が成功した後にチャネル送信で失敗する。呼び出し順の誤りが AMF の状態を変えてから検出されるため、原因が分かりにくい。

`Drop` はワーカーの終了を待つが、`cmd_tx` を切断した後に保留中の `user_data` を無通知で捨てる。また、`handle.join()` の結果を `let _ =` で捨てているため、ハンドラー内の panic が利用者に伝わらない。

## 現状

- `src/encode.rs` / `src/decode.rs` の `finish()` は `WorkerCommand::Finish` を送り、`finish` フィールドを `Some` にしてワーカーの終了を待つ。
- `encode()` / `decode()` は `finish` 済みかどうかを確認せず、AMF の `SubmitInput` を呼んでからチャネル送信する。
- `Drop` は `cmd_tx` を drop してから `join` するが、保留中の `pending` の `user_data` はハンドラーに渡されない。
- `Drop` の `let _ = handle.join();` は、ハンドラーが panic した場合の `Err` を握り潰す。
- `finish()` を 2 回呼んだ場合の契約が doc にない。

## 設計方針

- `finished` / `finishing` の状態を明示し、`encode()` / `decode()` / `finish()` の入口で状態を検証する。`finish` 済みなら AMF を呼ばずに即エラーを返す。
- `finish()` は冪等にする。2 回目はワーカーの終了を待って `Ok` を返す、または「すでに終了済み」を表すエラーを返すかを決めて doc に書く。
- `Drop` の契約を doc に書く。保留中の `user_data` を通知できないことを明記し、必要なら `Drop` の前に `finish()` を呼ぶよう案内する。
- ハンドラーの panic を `finish()` の戻り値に伝播する。`Drop` からは panic を再送出せず、ログまたは `std::process::abort` のどちらにするかを決める。
- 状態遷移のテストを追加する。`finish` 後の `encode` / `decode`、`finish` の二重呼び出し、保留中の `user_data` がある状態での `Drop` を検証する。

## 完了条件

- `finish` 後に `encode` / `decode` を呼んでも AMF を呼ばず、明確なエラーになる。
- `finish` の二重呼び出しの挙動が doc とテストで一致する。
- `Drop` の `user_data` の扱いと panic の扱いが doc に書かれている。
- 状態遷移のテストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。
