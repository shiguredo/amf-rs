# `Encoder` / `Decoder` の `finish` と `Drop` の状態を明確にする

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-finish-state
- Polished: {YYYY-MM-DD}
- Refreshed: 2026-11-18
- Reporter: @melpon

## 目的

`Encoder::finish()` の後に `Encoder::encode()` を呼ぶと、finish 済みかどうかを確認しないまま AMF の `SubmitInput` を呼ぶ。ワーカーが既に終了していればチャネル送信で失敗し、ワーカーがまだ終了していなければ送信は成功するが `user_data` は通知されない。呼び出し順の誤りが AMF の状態を変えてから、または何も起きずに見過ごされるため、原因が分かりにくい。

`Drop` はワーカーの終了を待つが、`cmd_tx` を切断した後にワーカーが保持する `user_data` と出力を無通知で捨てる。また、`handle.join()` の結果を `let _ =` で捨てているため、ハンドラー内の panic が利用者に伝わらない。

## 現状

`Decoder` は `finish` 済みの状態を持ち、入口で検証している。

- `Decoder::finish()` は `Drain` に成功した後に `cmd_tx` を `None` にしてから `WorkerCommand::Finish` を送る。
- `Decoder::decode()` と `Decoder::finish()` は入口で `cmd_tx` が `None` かを確認し、`finish` 済みなら AMF を呼ばずに「decoder is already finished」の `Err` を返す。2 回目の `finish` が `Err` になることは doc にも書かれている。
- これらの挙動は `test_decode_after_finish_is_rejected` と `test_finish_twice_is_rejected` で検証されている。
- `Decoder::decode()` の doc には `# Errors` がなく、`finish` 後の拒否も書かれていない。

`Encoder` は `finish` 済みの状態を持たない。

- `Encoder::finish()` は `cmd_tx` を保持したまま `Drain` を呼び、`WorkerCommand::Finish` の完了を待つ。2 回目の `finish()` は `Drain` を再度呼び、その後に `Err` を返す。二重呼び出しの契約は doc にない。
- `Encoder::encode()` は `finish` 済みかを確認せず、AMF の `SubmitInput` を呼んでからチャネル送信する。
- `Encoder::reconfigure()` も `finish` 済みかを確認せず、AMF のプロパティを設定する。
- `Encoder` の `finish` 後の挙動は `skills/shiguredo-amf/SKILL.md` に書かれていない。

両方に共通する問題。

- `Drop` は保留中の `user_data` と、ワーカーが保持する `output_buffer` をハンドラーに通知しない。
- `Drop` の `let _ = handle.join();` は、ハンドラーが panic した場合の `Err` を握り潰す。`let _ = self.component.terminate();` も結果を捨てている。

## 設計方針

- `cmd_tx: Option<Sender<...>>` を「`finish` 済み」の唯一の状態とする。`finished` のような別のフィールドは持たない。
- `finish()` は `Drain` に成功してから `cmd_tx` を `take()` する。`Drain` に失敗した場合は状態を変えずに `Err` を返し、呼び出し側が `finish()` をやり直せるようにする。`Drain` 成功後の送信または完了待ちに失敗した場合は `finish` 済みのままにする。
- `Encoder::encode()` / `Encoder::reconfigure()` / `Encoder::finish()` と `Decoder::decode()` / `Decoder::finish()` は入口で状態を検証し、`finish` 済みなら AMF を呼ばずに `Err` を返す。
- `finish()` の 2 回目の呼び出しは `Err` を返す。冪等にはしない。現在の `Decoder` と同じ契約に揃える。
- `finish()` はワーカーの終了を `join` まで待つ。ハンドラーが panic した場合は、panic の内容をエラーに含めずに `Err` を返す。
- `Drop` の契約を doc に書く。`finish()` を呼ばずに drop した場合、投入済みの入力に対する通知と、取得済みの出力の通知は行われない。
- `Drop` はハンドラーの panic を再送出せず、`tracing::error!` で記録する。unwinding 中の二重 panic を避けるため。
- `Drop` の `Component::terminate()` の失敗も `tracing::error!` で記録する。戻り値を返せない後始末であり、利用者に伝える手段がない。
- `Encoder::finish()` の doc にある「ワーカーが既に終了している場合や Drain に失敗した場合は `Err` を返す」に、`finish` 済みの場合を加える。
- `Decoder::decode()` の doc に `# Errors` を追加し、`finish` 後の拒否を書く。`Encoder::encode()` / `Encoder::reconfigure()` / `Encoder::finish()` にも同じ粒度で書く。
- `finish` 後の `stats()` やシーケンスパラメータの読み取りのように、AMF の状態を変えない操作は状態検証の対象にしない。検証するのは AMF の状態を変える `encode()` / `reconfigure()` と、ワーカーの終了を要求する `finish()` に限る。
- ワーカーを共通化してもこの契約を維持する。
- 対象外: `Decoder::flush` は公開 API に存在しないため扱わない。

## 完了条件

- `finish` 後の `Encoder::encode()` / `Encoder::reconfigure()` / `Encoder::finish()` が AMF を呼ばずに `Err` を返す。
- 2 回目の `finish()` が `Err` を返す契約が `Encoder` と `Decoder` の doc とテストで一致する。
- `Drop` の `user_data` と `output_buffer` の扱い、および panic の扱いが doc に書かれている。
- `Drop` がハンドラーの panic を `tracing::error!` で記録する。
- `Encoder` の状態遷移のテストが追加され、保留中の入力がある状態で `Decoder` / `Encoder` を `Drop` した場合に通知がないことも検証される。
- `cargo test --workspace` が通る。
- `Encoder::encode()` / `Encoder::reconfigure()` / `Encoder::finish()` と `Decoder::decode()` / `Decoder::finish()` の doc に `# Errors` が追加され、`finish` 後の拒否が書かれている。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。`Decoder` の `finish` 後の `decode` 拒否は記載済みなので、`Encoder` の状態検証と `Drop` の契約を追記する。
- `skills/shiguredo-amf/SKILL.md` のエンコーダー側の記述に `finish` 後の入力拒否を追記する。
