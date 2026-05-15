# 0001: 別インスタンス間のスレッド安全性欠如による SIGSEGV

Created: 2026-05-02
Completed: 2026-05-02
Model: deepseek-v4-pro

## 背景

AMF ランタイム（`libamfrt64.so.1`）はプロセス全体で単一のシングルトン `AMFFactory` を保持する。
`AMFInit()` は毎回同じ `AMFFactory*` を返す。

現在の `amf-rs` は `unsafe impl Send` のみを実装し `Sync` は実装しないことで
同一インスタンスへの同時アクセスを型レベルで防止しているが、
別々のインスタンスに対する異なるスレッドからの並行アクセスは防止できていない。

この結果、`cargo test` のデフォルト並列実行で SIGSEGV が発生する。
現状は `tests/test_roundtrip.rs` と `tests/test_codec_info.rs` で `AMF_LOCK` を使って
テストを直列化することで回避しているが、本番コード側には排他制御が存在しない。

## 再現方法

任意の AMF 操作を別々のインスタンスに対して複数スレッドから同時に実行する。
最も高頻度で落ちるのは `supported_codecs()` の並列呼び出し。

## GDB / シグナルハンドラ + `libc::backtrace` + `addr2line` による特定

### 共通の C++ バックトレース (libamfrt64.so.1 内部)

```
AMFFactoryHelper::Init(wchar_t const*)    ← ここで SIGSEGV (常に同一箇所)
AMFGetMemoryTypeName()
amf::AMFDeviceImpl<amf::AMFDeviceVulkan>::AMFDeviceImpl(...)
amf::AMFDeviceVulkanImpl::AMFDeviceVulkanImpl(...)
amf::AMFCreateDeviceVulkan(...)
AMFContextImpl::InitVulkan(void*)
```

### supported_codecs() 経由の Rust バックトレース

```
supported_codecs()              @ src/codec_info.rs:119
  → ProbeContext::new()         @ src/codec_info.rs:230
    → AmfLibrary::init_vulkan() @ src/lib.rs:165
      → AMFContextImpl::InitVulkan → ... → SIGSEGV
```

### Encoder::new() 経由の Rust バックトレース

```
Encoder::new()                  @ src/encode.rs:378
  → AmfLibrary::init_vulkan()   @ src/lib.rs:165
    → AMFContextImpl::InitVulkan → ... → SIGSEGV
```

## 原因

1. **AMFFactory はプロセスシングルトン** — `AMFInit()` が返す factory ポインタは全インスタンスで同一。
2. **InitVulkan の内部で factory の共有状態にアクセスしている** — `AMFContextImpl::InitVulkan` が Vulkan デバイス作成時に `AMFFactoryHelper::Init` を呼び、factory 内部のグローバルな登録テーブルにアクセスする。
3. **この操作はスレッドセーフではない** — 複数スレッドが同時に `InitVulkan` → `AMFFactoryHelper::Init` を実行すると、内部のポインタやハッシュテーブル等が破壊され SIGSEGV となる。

以上より、問題の発生箇所は `AmfLibrary::init_vulkan()` に集約される。
複数スレッドが同時に異なる `AMFContext` に対して `InitVulkan` を呼ぶと、
内部で共通の `AMFFactoryHelper::Init` が走り競合する。

## AMF SDK ドキュメント調査結果

AMF API Reference (`AMF_API_Reference.md`) およびヘッダーファイルの確認結果:

- `Factory.h`: `// AMFFactory interface - singleton` と明記 → AMFFactory はプロセスシングルトン
- `AMF_API_Reference.md` §2.6.1: `All AMF components are thread-safe.` → AMFComponent (Encoder/Decoder) はスレッド安全
- `AMF_API_Reference.md` §2.2.14.1: `The default implementation is not thread-safe.` → AMFPropertyStorage はスレッド安全ではない
- `AMF_API_Reference.md` §2.2.14.3: `The default implementation is not thread-safe.` → AMFPropertyStorageEx も同様
- `AMF_API_Reference.md` §2.3.1: `AMFData objects are generally not thread-safe.` → AMFData はスレッド安全ではない
- `AMF_API_Reference.md` §2.3.3.1: `AMFSurface objects are generally not thread-safe.` → AMFSurface も同様
- `AMFInit` および `AMFFactoryHelper::Init(Vulkan)` については明示的なスレッド安全性の記述が存在しない

## 解決方法

### 設計判断

1. `AMFInit` が毎回同じ factory ポインタを返すこと、および `AMFFactory` が仕様上 singleton であることから、`AmfLibrary` をプロセス全体で単一のインスタンスとするシングルトンパターンが適切である。
2. `AMFInit`（初回のライブラリロード）と `InitVulkan` は内部で共通の `AMFFactoryHelper::Init` を操作するため、同一の排他制御で直列化する。

### 実装方式

`AmfLibrary` を `LazyLock<AmfLibrary>` でプロセスシングルトンとし、内部に `Mutex<Option<AmfLibraryInner>>` を持つ。

- `LazyLock` が `AmfLibrary` のインスタンス化を排他制御しつつ 1 回だけ行う（`AmfLibrary::new()` は単に `Mutex::new(None)` するだけなので競合しない）
- `AmfLibraryInner` が実際の `DynLib` ハンドルと `AMFFactory` ポインタを保持する
- `ensure_inner()` が `Mutex<Option<AmfLibraryInner>>` のロック下で遅延初期化（dlopen + AMFInit）を行う。2 回目以降はキャッシュされた値を返す
- `factory_ptr()`, `create_context()`, `query_version()` はいずれも `self.inner.lock()` を取得した上で `ensure_inner()` を呼び、AMFInit が完了していることを保証する
- `create_component()` も同様に `self.inner.lock()` を取得し、factory ポインタをロック外に公開せず安全に CreateComponent を実行する
- `init_vulkan()` も同一の `self.inner.lock()` を取得するため、AMFInit と InitVulkan は同一の排他制御で直列化される

この方式により:
- ロックは `Mutex<Option<AmfLibraryInner>>` の 1 つだけ
- Double-Checked Locking は発生しない（`ensure_inner` の `is_none()` チェックは常にロック下で行われる）
- `instance()` は `&'static AmfLibrary` を返し infallible（`LazyLock` がインスタンスの存在を保証する）

### 修正内容

1. **`AmfLibrary` をシングルトン化** (`src/lib.rs`)
   - `static AMF_LIBRARY: LazyLock<AmfLibrary> = LazyLock::new(AmfLibrary::new)` を追加
   - `AmfLibrary` は `inner: Mutex<Option<AmfLibraryInner>>` を保持
   - `AmfLibraryInner` が `DynLib` と `*mut AMFFactory` を持つ
   - `instance()` は `&'static AmfLibrary` を返す
   - `AmfLibrary` に `unsafe impl Sync` を追加

2. **AMFInit と InitVulkan を同一の排他制御で直列化** (`src/lib.rs`)
   - `ensure_inner()` が `Mutex<Option<AmfLibraryInner>>` のロック下で dlopen + AMFInit を行う
   - `init_vulkan()` も同一の `self.inner.lock()` を取得し、InitVulkan → AMFFactoryHelper::Init を排他制御下で実行
   - `init_vulkan()` は関連関数から `&self` インスタンスメソッドに変更

3. **Encoder/Decoder/ProbeContext から `_lib` フィールドを削除し、`create_component()` 経由でアクセス**
   - `src/encode.rs`, `src/decode.rs`, `src/codec_info.rs`
   - ライブラリハンドルは `static AMF_LIBRARY` が永続的に保持するため不要
   - factory ポインタを公開する `factory_ptr()` は Mutex の安全性を壊すため削除し、代わりにロック内で CreateComponent を実行する `create_component()` を追加
   - 呼び出し側は `let lib = AmfLibrary::instance();` で取得し、以降 `lib.create_context()`, `lib.create_component(context, id)`, `lib.init_vulkan(context)` のように使う

4. **テストの `AMF_LOCK` を削除**
   - `tests/test_roundtrip.rs` (24 テスト), `tests/test_codec_info.rs` (2 テスト)
   - 本番コード側で排他制御が実装されたため不要になった

### API 変更

- `AmfLibrary::load()`（戻り値 `Result<Self, Error>`）を削除
- `AmfLibrary::instance()`（戻り値 `&'static AmfLibrary`）を追加。`LazyLock` により infallible
- `AmfLibrary::factory_ptr()` を削除。factory ポインタをロック外に公開すると Mutex の安全性が壊れるため
- `AmfLibrary::create_component(context, component_id)`（戻り値 `Result<*mut AMFComponent, Error>`）を追加。factory の CreateComponent をロック内で安全に実行する
- `AmfLibrary::init_vulkan()` を関連関数から `&self` インスタンスメソッドに変更
- `AmfLibrary::query_version()` は従来通り `&self` インスタンスメソッドを維持
