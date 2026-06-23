# Encoder/ Decoder のコールバック関数をトレイトベースのハンドラーに変更する

Created: 2026-05-14
Completed: 2026-05-14
Model: deepseek-v4-pro

## 背景と根拠

現在の `Encoder<T>` / `Decoder<T>` はコールバック関数 (`FnMut`) を直接受け取る方式を採用している。
トレイトベースにすることで以下の利点が得られる:

- 関連型 (`UserData`, `Error`) でユーザーデータ型とエラー型を明示的に定義できる
- エラーハンドリングをコールバック内で行えるようになる (`Result` を受け取る形にすることで、AMF 内部エラーもハンドラー側で処理できる )
- 後方互換性のために `FnMut` ラッパー (`FnEncodeHandler` / `FnDecodeHandler`) を提供することで、既存のクロージャベースの利用も引き続き可能にする

## 要件

### 1. EncodeHandler トレイトの導入 (`src/encode.rs` )

```rust
/// エンコード結果を通知するためのハンドラー
///
/// エンコード処理が完了するたびに [`EncodeHandler::on_encoded`] が呼ばれる。
pub trait EncodeHandler: Send + 'static {
    /// ユーザーデータ型
    type UserData: Send + 'static;
    /// エラー型
    type Error: From<crate::Error> + Send + 'static;
    /// エンコード完了時に呼ばれる
    fn on_encoded(&mut self, result: Result<EncodedFrame<Self::UserData>, Self::Error>);
}
```

### 2. DecodeHandler トレイトの導入 (`src/decode.rs` )

```rust
/// デコード結果を通知するためのハンドラー
///
/// デコード処理が完了するたびに [`DecodeHandler::on_decoded`] が呼ばれる。
pub trait DecodeHandler: Send + 'static {
    /// ユーザーデータ型
    type UserData: Send + 'static;
    /// エラー型
    type Error: From<crate::Error> + Send + 'static;
    /// デコード完了時に呼ばれる
    fn on_decoded(&mut self, result: Result<DecodedFrame<Self::UserData>, Self::Error>);
}
```

### 3. FnEncodeHandler ラッパーの導入 (`src/encode.rs` )

`FnMut(Result<EncodedFrame<T>, E>)` を `EncodeHandler` にするラッパー構造体を提供する。
`Box<dyn FnMut>` によるヒープ割り当てと動的ディスパッチが発生するが、
エンコード/ デコードの完了はフレームレートに律速される低頻度イベントであるため、性能影響は無視できる。

```rust
pub struct FnEncodeHandler<T, E = crate::Error> {
    f: Box<dyn FnMut(Result<EncodedFrame<T>, E>) + Send + 'static>,
}

impl<T, E> FnEncodeHandler<T, E> {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(Result<EncodedFrame<T>, E>) + Send + 'static,
    {
        Self { f: Box::new(f) }
    }
}

impl<T, E> EncodeHandler for FnEncodeHandler<T, E>
where
    T: Send + 'static,
    E: From<crate::Error> + Send + 'static,
{
    type UserData = T;
    type Error = E;
    fn on_encoded(&mut self, result: Result<EncodedFrame<T>, E>) {
        (self.f)(result);
    }
}
```

### 4. FnDecodeHandler ラッパーの導入 (`src/decode.rs` )

`FnMut(Result<DecodedFrame<T>, E>)` を `DecodeHandler` にするラッパー構造体を提供する。
性能影響については `FnEncodeHandler` と同様。

```rust
pub struct FnDecodeHandler<T, E = crate::Error> {
    f: Box<dyn FnMut(Result<DecodedFrame<T>, E>) + Send + 'static>,
}

impl<T, E> FnDecodeHandler<T, E> {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(Result<DecodedFrame<T>, E>) + Send + 'static,
    {
        Self { f: Box::new(f) }
    }
}

impl<T, E> DecodeHandler for FnDecodeHandler<T, E>
where
    T: Send + 'static,
    E: From<crate::Error> + Send + 'static,
{
    type UserData = T;
    type Error = E;
    fn on_decoded(&mut self, result: Result<DecodedFrame<T>, E>) {
        (self.f)(result);
    }
}
```

### 5. Encoder の変更

#### 5-1. 構造体定義の変更

- `Encoder<T: Send + 'static>` → `Encoder<H: EncodeHandler>`
- `T` 型パラメータは削除し、代わりに `H` を使用する
- 変更対象フィールド:
  - `cmd_tx: Option<mpsc::Sender<WorkerCommand<T>>>` → `cmd_tx: Option<mpsc::Sender<WorkerCommand<H::UserData>>>`
 - その他のフィールドは変更不要

#### 5-2. Encoder::new() の変更

- 引数: `callback: impl FnMut(EncodedFrame<T>) + Send + 'static` → `handler: H`
- ワーカースレッド起動時に `handler` をムーブする
- `impl<T: Send + 'static> Encoder<T>` → `impl<H: EncodeHandler> Encoder<H>`

#### 5-3. Encoder::encode() のシグネチャ変更

- `encode(&mut self, surface: Surface, options: &EncodeOptions, user_data: T)` の `T` は `H::UserData` に変わる
- メソッド本体の変更は不要（`WorkerCommand::Submit(user_data)` で型が自然に追従する）

#### 5-4. その他メソッドの impl 境界変更

`Drop`、`reconfigure()`、`alloc_surface()` 等のメソッドは本体の変更不要だが、
impl ブロックの型境界が `impl<T: Send + 'static>` から `impl<H: EncodeHandler>` に変わる。

### 6. Decoder の変更

Decoder も Encoder と同様の変更を行う。`decode()` メソッドの `user_data` 引数は `H::UserData` に変わるが、その他は Encoder の各変更を `EncodeHandler` → `DecodeHandler` と読み替えればよい。

### 7. ワーカースレッドの変更

#### 7-1. WorkerCommand の型パラメータ変更

`encode.rs` / `decode.rs` ともに `WorkerCommand<T>` の `T` はそのまま維持する。
呼び出し側で `T = H::UserData` と具体化されるため、型パラメータ名の変更は不要。

#### 7-2. worker 関数のシグネチャ変更

```rust
// before (encode.rs:937)
fn worker<T, F>(
    component: Component,
    mut callback: F,
    cmd_rx: mpsc::Receiver<WorkerCommand<T>>,
    codec_config: CodecConfig,
) where
    T: Send + 'static,
    F: FnMut(EncodedFrame<T>) + Send + 'static,
{ ... }

// after
fn worker<H: EncodeHandler>(
    component: Component,
    mut handler: H,
    cmd_rx: mpsc::Receiver<WorkerCommand<H::UserData>>,
    codec_config: CodecConfig,
) { ... }
```

`decode.rs` 側も同様に:

```rust
fn worker<H: DecodeHandler>(
    component: Component,
    mut handler: H,
    cmd_rx: mpsc::Receiver<WorkerCommand<H::UserData>>,
) { ... }
```

#### 7-3. drain_output 関数のシグネチャ変更

`output_buffer` は `drain_output` 呼び出し間で未マッチ出力を蓄積する状態であり、
引数として引き続き渡す。

**encode.rs 側**:

```rust
// before (encode.rs:996)
fn drain_output<T, F>(
    output_buffer: &mut VecDeque<(Buffer, PictureType)>,
    pending: &mut VecDeque<T>,
    callback: &mut F,
    component: &Component,
    codec_config: &CodecConfig,
) -> Result<(), Error>
where
    T: Send + 'static,
    F: FnMut(EncodedFrame<T>),
{ ... }

// after
fn drain_output<H: EncodeHandler>(
    output_buffer: &mut VecDeque<Result<(Buffer, PictureType), crate::Error>>,
    pending: &mut VecDeque<H::UserData>,
    handler: &mut H,
    component: &Component,
    codec_config: &CodecConfig,
) { ... }
```

**decode.rs 側**（抽出対象が `Surface` になり、`codec_config` 引数は従来通り不要）:

```rust
// after
fn drain_output<H: DecodeHandler>(
    output_buffer: &mut VecDeque<Result<Surface, crate::Error>>,
    pending: &mut VecDeque<H::UserData>,
    handler: &mut H,
    component: &Component,
) { ... }
```

変更点:
- `callback` 引数を `handler` に変更
- `output_buffer` の要素型を `Result<...>` に変更し、抽出エラーをマーカーとして格納できるようにする
- 戻り値型 `Result<(), Error>` を廃止し `()` にする（エラーは `output_buffer` 経由で伝搬する）
- `callback(EncodedFrame { ... })` → `handler.on_encoded(Ok(EncodedFrame { ... }))`
- `decode.rs` の `drain_output` は現在 `codec_config` 引数を持たない。本変更後も不要なため引数追加はしない。

#### 7-4. worker 内の drain_output 呼び出し変更

`output_buffer` は worker のループ外で宣言し、呼び出し間で状態を持ち越す（位置は従来通り）。

**encode.rs** (`encode.rs:946-978` 付近):

```rust
// before
if let Err(e) = drain_output(
    &mut output_buffer,
    &mut pending,
    &mut callback,
    &component,
    &codec_config,
) {
    log::error!("worker: drain_output failed: {e}");
}

// after
drain_output(
    &mut output_buffer,
    &mut pending,
    &mut handler,
    &component,
    &codec_config,
);
```

**decode.rs** も同様に戻り値チェックを除去し、引数順を合わせる。

**既知の制限**: `Finish` 受信時に `pending` が空だと `drain_output` が呼ばれずにループを抜けるため、`output_buffer` に残留データがある場合に取りこぼされる可能性がある。これは既存実装と同一の振る舞いであり、本変更の対象外とする。

#### 7-5. 抽出エラーのハンドラー通知

`drain_output` 内で `extract_encoded_output` (encode.rs:1018-1021) や
`extract_frame` (decode.rs:296-300, 313) が失敗した場合、
現状は `log::error!` で破棄されている。

トレイト導入後は、抽出エラーをハンドラーに `Err` として通知する。
FIFO 順序を保つため、QueryOutput ループ内で pending に直接触れるのではなく、
`output_buffer` にエラーマーカーとして `Err` を格納し、
後続のマッチングループで順序通りに `handler.on_encoded(Err(err.into()))` を呼ぶ方式とする。

具体的な制御フロー:

1. **QueryOutput ループ**:
   - 抽出成功 → `output_buffer.push_back(Ok((buffer, picture_type)))`
   - 抽出失敗 → `output_buffer.push_back(Err(err))`
2. **マッチングループ**:
   - `output_buffer.pop_front()` → `Ok((buffer, pt))` → `pending.pop_front()` → `handler.on_encoded(Ok(EncodedFrame { ... }))`
   - `output_buffer.pop_front()` → `Err(err)` → `pending.pop_front()` → `handler.on_encoded(Err(err.into()))`

これにより QueryOutput からの出力順と pending 内の user_data 順が常に一致する。
抽出エラー発生時も pending から 1 要素をポップすることで対応関係を維持する。

この方式は、現在の実装において 1 回の `SubmitInput` が常に 1 回の `QueryOutput` 出力に対応するという前提に基づく。

decode.rs 側も同様に、`extract_frame` のエラーを `output_buffer` に `Err` として格納する。
抽出コードパスは 2 箇所ある:

1. `AMF_REPEAT` + 非 null `data` パス (`decode.rs:296-299`): `log::error!` → `output_buffer.push_back(Err(e))` / `continue`
2. `AMF_OK` パス (`decode.rs:311-313`): `log::error!` → `output_buffer.push_back(Err(e))` / ループ継続

いずれも同じ方式（`output_buffer` に `Err` を格納）で扱う。

### 8. 既存のコールバック利用箇所の移行

テスト (`tests/test_roundtrip.rs`) で `Encoder::new()` / `Decoder::new()` を呼んでいる箇所を
`FnEncodeHandler::new(...)` / `FnDecodeHandler::new(...)` に置き換える。

デフォルトエラー型 (`crate::Error`) を利用するため、型注釈は最小限で済む。

**before/ after 例**:

```rust
// before (test_roundtrip.rs:283)
let mut encoder = Encoder::new(config, move |frame: EncodedFrame<()>| {
    r.lock().unwrap().push(frame);
})

// after
let mut encoder = Encoder::new(config, FnEncodeHandler::new(move |result| {
    r.lock().unwrap().push(result.unwrap());
}))
```

テスト内の全 `Encoder::new` / `Decoder::new` 呼び出し箇所を同様に変更する（`rg "Encoder::new\|Decoder::new" tests/` で全箇所を検索できる）。

### 9. 公開 API のエクスポート (`src/lib.rs` )

`lib.rs` の `pub use` に以下を追加する:

- `EncodeHandler`, `FnEncodeHandler`
- `DecodeHandler`, `FnDecodeHandler`

### 10. テスト戦略

#### 10-1. 単体テスト

`FnEncodeHandler` のテストは既存の `tests/test_encode.rs` に追加する
（`EncodeHandler` は `src/encode.rs` に定義されるため、CLAUDE.md の命名規則 `tests/test_<module>.rs` に従う）。
`FnDecodeHandler` のテストは `tests/test_decode.rs` を新規作成して追加する。

- `FnEncodeHandler::new()` でラップしたクロージャが `EncodeHandler::on_encoded()` を通して正しく呼ばれること
- `FnDecodeHandler::new()` でラップしたクロージャが `DecodeHandler::on_decoded()` を通して正しく呼ばれること

#### 10-2. PBT

ハンドラーの核心は型安全性にあるため、PBT は既存のラウンドトリップテスト (`test_roundtrip.rs`) に `FnEncodeHandler` / `FnDecodeHandler` 経由のテストが移行されることで十分カバーされる。新規 PBT は不要。

#### 10-3. Fuzzing

`EncodeHandler::on_encoded` / `DecodeHandler::on_decoded` への任意入力に対する耐性は
fuzzing の対象になりうるが、ハンドラー自体はユーザー定義であり、
内部実装でパニックを起こす経路はないため、新規 fuzzing ターゲットの追加は不要。

### 11. 変更履歴

`CHANGES.md` の `## develop` セクションに以下のエントリを**新規追加**する（既存の `[CHANGE]` エントリの末尾に追記）:

```
- [CHANGE] Encoder/ Decoder のコールバックをトレイトベースのハンドラーに変更する
  - @melpon
```

## 実装手順

1. `EncodeHandler` + `FnEncodeHandler` を `src/encode.rs` に定義する
2. `DecodeHandler` + `FnDecodeHandler` を `src/decode.rs` に定義する
3. `encode.rs` の `Encoder`、`worker`、`drain_output` の型パラメータを変更する
4. `drain_output` 内のエラーハンドリングを 7-5 節に従って変更する（QueryOutput ループ・マッチングループ）
5. `cargo build` でエンコーダー側のコンパイルを通す
6. `decode.rs` の `Decoder`、`worker`、`drain_output` の型パラメータを変更する
7. デコーダー側の `drain_output` 内エラーハンドリングを 7-5 節に従って変更する
8. `cargo build` で全体のコンパイルを通す
9. `lib.rs` の `pub use` を更新する
10. テストを移行する（`rg "Encoder::new\|Decoder::new" tests/` で特定した全箇所）
11. `cargo test` で全テストが通過することを確認する
12. CHANGES.md を更新する

## 解決方法

issue の要件どおりに実装した。主な変更点:

- `src/encode.rs`: `EncodeHandler` トレイトと `FnEncodeHandler` ラッパーを追加し、`Encoder<T>` を `Encoder<H: EncodeHandler>` に変更した。`worker`/`drain_output` の型パラメータも同様に変更し、`drain_output` の `output_buffer` を `Result` 型に変更して抽出エラーをハンドラーに `Err` として通知するようにした
- `src/decode.rs`: `DecodeHandler` トレイトと `FnDecodeHandler` ラッパーを追加し、`Decoder<T>` を `Decoder<H: DecodeHandler>` に変更した。エラーハンドリングも encode 側と同様に変更した
- `src/lib.rs`: `EncodeHandler`、`FnEncodeHandler`、`DecodeHandler`、`FnDecodeHandler` をエクスポートに追加した
- `tests/test_roundtrip.rs`: 全 `Encoder::new`/`Decoder::new` 呼び出しを `FnEncodeHandler`/`FnDecodeHandler` 経由に移行した
- `CHANGES.md`: `[CHANGE]` エントリを追加した
