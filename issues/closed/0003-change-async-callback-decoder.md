# Decoder の出力取得を非同期コールバック方式に変更する

Created: 2026-05-03
Completed: 2026-05-03
Model: deepseek-v4-pro

## 背景

現在の `Decoder` は `next_frame()` を呼び出してデコード済みフレームを取得するポーリング方式を採用している。
しかし、デコード完了の情報を取得するタイミングが 1 フレーム遅れるため、映像データ受信の遅延に繋がっている。

AMFComponent の関数はスレッドセーフであると AMF ドキュメントに記載されており、
別スレッドから QueryOutput を呼び出して結果をコールバックで受け取ることが可能である。

Encoder 側は既に非同期コールバックに対応しているため、この実装を参考にする。

## 要件

- `Decoder::new()` でコールバック用の関数を渡し、デコードが完了する度にその関数を呼ぶ
- 新しくスレッドを起こして、その中で `AMFComponent::QueryOutput` を呼び出し、結果が得られたらそのスレッドの中でコールバックを呼び出す
- `AMFComponent::QueryOutput` はノンブロッキングの関数であるため、デコードキューが空の場合は無駄に呼び出さないようにする (pending キューが空なら `recv()` で完全待機、非空なら `recv_timeout(1ms)` で 1ms 間隔のポーリング)
- `Decoder::decode()` 時に任意の値 `T` を渡せるようにし、入力データに対応した出力フレームが得られた時に、コールバックにその値を渡す
- `next_frame()` は不要になるため削除する

## 設計

### Decoder 構造体

`Decoder<T: Send + 'static>` としてジェネリックにし、以下のフィールドを追加する:

- `cmd_tx: Option<mpsc::Sender<WorkerCommand<T>>>` — ワーカースレッドへのコマンド送信用
- `poll_thread: Option<JoinHandle<()>>` — ワーカースレッドのハンドル

### WorkerCommand

```rust
enum WorkerCommand<T> {
    Submit(T),
    Finish(mpsc::SyncSender<()>),
}
```

チャネル切断 (= Sender が drop) で Stop 指示とする。

### ワーカースレッド

 1. pending キュー (`VecDeque<T>`) が空なら `recv()` で完全待機
 2. pending キューが非空なら `recv_timeout(1ms)` で 1ms 待機
 3. タイムアウトしたら `QueryOutput` を呼ぶ
 4. 結果は一旦 `output_buffer` (`VecDeque<DecodedFrame>`) に格納し、pending とマッチングできた分だけコールバックを呼び出す
     - `QueryOutput` の出力が `Submit(T)` より先に到着する競合があるため、直接 pop せずバッファリングして後方マッチングする
 5. `Submit(T)` コマンド受信時は pending に push
 6. `Finish(tx)` コマンド受信時は pending が空になるまで QueryOutput ループ、完了したら tx.send(())

### decode()

SubmitInput 成功後に `cmd_tx.send(WorkerCommand::Submit(user_data))` を送信。
SubmitInput 失敗時はチャネルに送信していないので user_data はドロップされる。

### finish()

1. `Drain` を呼ぶ
2. `Finish` コマンドを送り、全 pending が処理されコールバックが呼ばれるのを待つ

### Drop

1. `cmd_tx` を None にし Sender を drop (= チャネル切断)
2. `poll_thread.take()` → join
3. 従来の component/context 解放

### extract_frame の standalone 化

- `Decoder` のメソッドから独立関数に変更する
- `self.decoded_frames.push_back(...)` の代わりに戻り値 `Option<DecodedFrame>` を返す
- エラー時は release_surface して `None` を返す

## 解決方法

- `Decoder<T: Send + 'static>` に汎化し、`Decoder::new()` にコールバックを受け取る引数を追加した
- mpsc チャネル + `WorkerCommand<T>` でスレッド間通信を実装し、ワーカースレッド内で `QueryOutput` をポーリングする方式に変更した
- `decode()` に `user_data: T` パラメータを追加し、対応する出力が得られたときにコールバックへ渡すようにした
- `next_frame()` と `poll_output()` を削除した
- `finish()` は `Drain` → `Finish` コマンド送信 → SyncSender で完了待機する方式とした
- `Drop` で `cmd_tx` 切断 → `poll_thread.join()` → component/context 解放の順に停止する
- `QueryOutput` の出力が `Submit(T)` より先に到着する競合に対処するため、`output_buffer` (`VecDeque<DecodedFrame>`) を導入してバッファリングし、pending と後方マッチングする方式とした
- `extract_frame()` を standalone 関数に変更し、ワーカースレッドから呼び出せるようにした
- `AMF_REPEAT` で data が非 null の場合の処理を `drain_output` に追加した（旧 `poll_output` 相当の動作）
- テストの `decode` ヘルパーをコールバック方式に書き換えた
- `DecodedFrame` に `Debug` derive を追加した
- README.md のデコードサンプルをコールバック方式に更新した
- CHANGES.md に変更エントリを追加した
