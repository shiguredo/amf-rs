# `Error` を分類 enum に変更する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-error-design
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Error` が単一の struct で、すべての失敗を `Error::new_custom` と `Error::from_amf` で表現している。利用者は `status()` の `None` とメッセージ文字列から失敗の種類を推測するしかなく、設定不正、API の誤用、AMF の失敗、環境障害、内部の前提違反を型で区別できない。

失敗の分類を enum と分類型で表し、標準エラーからの変換を `From` に寄せる。`source()` で内側の標準エラーを取り出せるようにし、`map_err` の重複を減らす。

## 現状

- `src/error.rs` の `Error` は `status: Option<AMF_RESULT>`、`function: Cow<'static, str>`、
  `message: String` を持つ struct で、`new_custom` / `from_amf` / `check` / `status` が
  public になっている。分類を型で強制できず、`status() == Some(AMF_OK)` のようなエラーも作れる。
- `STATUS_TABLE` が `AMF_RESULT` の variant と名前・説明を二重管理している。`AMF_RESULT` は 46 variant あるが、表は 38 個しか持たず、`AMF_SURFACE_MUST_BE_SHARED`、`AMF_DEM_*`、`AMF_TAN_*` の 8 個が漏れている。
- `Display` は `"{}() failed[status={:?}]: {}"` とメッセージ内の `({name})` で status 名を 2 回出している。
- `new_custom` は `function: &'static str`、`from_amf` は `impl Into<Cow<'static, str>>` を受け取り、型が統一されていない。
- テストは `err.status() == None` を「ライブラリ側のエラー」の意味で使っている (`test_encoder_rejects_hevc_10bit_without_main10`、`test_encoder_rejects_hevc_qpb`、`test_reconfigure_hevc_rejects_qpb_and_gop`)。型の契約ではなく慣習に依存している。
- `src` 内の `map_err` が 18 箇所あり、ワーカースレッドの終了、スレッド起動、動的ライブラリのロード、`CString` の変換、整数変換で同じようなエラー生成を繰り返している。`ok_or_else` は 3 箇所ある。
- `build.rs` は `AMF_RESULT` を bindgen の `rustified_enum` で生成している。AMF が未知の status を返した場合、FFI 境界の時点で不正な enum 値になり、Rust 側で元の値を保持できない。

## 設計方針

### `Error` の分類

`Error` を次の 5 分類の enum にする。

```rust
/// AMF 操作のエラー
#[derive(Debug)]
pub enum Error {
    /// 利用者が渡した値が API の契約を満たさない
    InvalidArgument(InvalidArgumentError),
    /// オブジェクトの状態が操作に適していない
    InvalidState(InvalidStateError),
    /// AMF の呼び出しがエラー status を返した
    Amf(AmfError),
    /// 実行環境に起因する失敗
    Environment(EnvironmentError),
    /// ライブラリまたは AMF の前提が破られた
    Internal(InternalError),
}
```

- `InvalidArgument`: 設定値や引数が API の契約を満たさない。値を直して再試行できる。
- `InvalidState`: オブジェクトの状態が操作に適していない。呼び出し順の誤り。
- `Amf`: AMF の呼び出しがエラー status を返した。`AMF_NOT_SUPPORTED` や `AMF_INVALID_FORMAT` のような未対応もここに含め、ライブラリ側で status を解釈し直さない。
- `Environment`: 動的ライブラリのロード、シンボル取得、ワーカースレッド起動など、実行環境に起因する失敗。
- `Internal`: AMF が `AMF_OK` を返したのに null を返す、vtable の関数ポインタが欠けている、ワーカースレッドが予期せず終了する、出力と入力の対応付けができない、などの前提違反。

`#[non_exhaustive]` は使わないため、分類の追加は破壊的変更になる。5 分類を固定し、増やす場合は `CHANGES.md` の `[CHANGE]` として扱う。

### 分類型

分類型は public にし、フィールドは private、コンストラクタは `pub(crate)` にする。利用者は `Error` の variant を match して分類型を取り出せるが、内部表現には触れられない。

- 呼び出し元の Rust 関数名は保持しない。同期 API のエラーでは呼び出し側が操作を知っているため、情報量が少ない。
- AMF の関数名 (`AMFComponent::Init`、`AMFComponent::SubmitInput`、`AMFContext::AllocSurface` など) は `AmfError::operation` として保持する。同じ Rust 関数から複数の AMF 呼び出しがあるため、どの呼び出しが失敗したかは残す価値がある。
- `AmfError` は `status: AMF_RESULT`、`operation: Cow<'static, str>`、必要なら補足メッセージを持つ。`status()` と `operation()` を公開する。`AMF_OK` はエラーではないため、`AmfError::new` で `assert_ne!` により拒否する。
- `AmfError::check(status, operation)` は `AMF_OK` なら `Ok(())`、それ以外は `Err(AmfError)` を返す。
- `EnvironmentError` は文脈メッセージと `source: std::io::Error` を持ち、`dlopen` / `dlsym` の失敗は `std::io::Error::other` で包む。
- `InternalError` は文脈メッセージと、必要に応じて `source: Box<dyn std::error::Error + Send + Sync>` を持つ。`NulError`、`TryFromIntError`、`RecvError` など複数種類の標準エラーを保持できるようにする。
- `InvalidArgumentError` と `InvalidStateError` は文脈メッセージだけを持つ。
- `Error` はハンドラー経由でワーカースレッドから渡されるため、`Send + Sync` を維持する。

### `From` と `source()`

- 分類型から `Error` への `From` を実装する。`?` で分類型を `Error` に上げられるようにする。
- 標準エラーから `Error` への `From` を実装し、`map_err` を減らす。
  - `std::io::Error` → `Environment`
  - `std::ffi::NulError` → `Internal`
  - `std::num::TryFromIntError` → `Internal`
  - `std::sync::mpsc::RecvError` → `Internal`
  - `std::sync::mpsc::SendError<T>` → `Internal` (`T` は保持せず、未送信のコマンドを捨てる。この場合は `source()` を返さない)
- `Error` に `std::error::Error` を実装し、`source()` は分類型の wrapper ではなく、内側の `std::io::Error` などを返す。`AmfError`、`InvalidArgumentError`、`InvalidStateError` は source を持たない。
- `Display` は source を持つ分類では文脈だけを出し、原因は `source()` に任せる。status 名は `Debug` から 1 回だけ出す。`STATUS_TABLE` は削除する。
- `Error::new_custom`、`Error::from_amf`、`Error::check`、`Error::status` は公開 API から削除する。
- `ok_or_else` は `Option` から `Result` への変換なので残す。`map_err` は `FrameError` に PTS を付ける 2 箇所と、`map_err(Into::into)` 1 箇所だけにする。

### 範囲外

- `AMF_RESULT` の未知の値の保持。`rustified_enum` を `newtype_enum` に変更する必要があり、FFI 全体に影響するため、この issue では扱わない。必要なら別 issue にする。

## 完了条件

- `Error` が 5 分類の enum になり、利用者が variant で失敗の種類を判別できる。
- 分類型のフィールドが private で、コンストラクタが `pub(crate)` になっている。
- `AmfError` が status と operation を保持し、`Display` が status 名を 1 回だけ出す。
- `STATUS_TABLE` が削除されている。
- 標準エラーから `Error` への `From` が実装され、`src` 内の `map_err` が `FrameError` の 2 箇所と `map_err(Into::into)` の 1 箇所だけになっている。
- `Error::source()` が `Environment` と `Internal` の内側の標準エラーを返す。
- テストが `err.status() == None` ではなく variant の match で分類を検証している。AMF の status を検証するテストは `Error::Amf` から `AmfError::status()` を読む。
- `Display` のテストが status 名の重複を検出する。
- `Error` と分類型の public な項目に doc が付き、`cargo test --workspace` と `cargo clippy --workspace --all-targets` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` として追記されている。
