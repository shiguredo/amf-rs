# `InitVulkan` を `AmfLibrary` の Mutex で直列化し、並列初期化時の SIGSEGV を修正する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-init-vulkan-serialization
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Context::init_vulkan` が `AmfLibrary` の Mutex を取らずに `AMFContext1::InitVulkan` を呼んでいる。`InitVulkan` の内部 (`AMFDeviceImpl<AMFDeviceVulkan>` の構築や `CreateCommandPool`) は `AMFFactoryHelper::Init` を呼び、この関数は非 thread-safe な遅延初期化を行う。ハンドルを非アトミックに確認し、`dlopen` とフィールドの書き込みを行うため、複数スレッドが同時に呼ぶと SIGSEGV する。

別々の `Context` に対して並列に `init_vulkan` を呼ぶだけで SIGSEGV することを実機で確認済みである (64 個の `Context` に対する並列 `init_vulkan` が 5 回中 5 回 SIGSEGV)。`AMFInit` は静的 factory を返すだけで `AMFFactoryHelper::Init` を呼ばないため、`AmfLibrary` 側のロックではなく `InitVulkan` 側のロックが必要である。

過去に SIGSEGV の修正として「`AMFInit` と `InitVulkan` を同一の Mutex で直列化する」設計が記録されているが、AMF オブジェクトの RAII ラッパー化によってそのロックが失われた回帰である。

## 現状

- `src/amf.rs` の `Context::init_vulkan` は `AmfLibrary` の Mutex を取らない。
- `src/lib.rs` の `AmfLibrary` の Mutex は `query_version`、`create_context`、`create_component` でのみ取得される。
- `Context::init_vulkan` の呼び出しは `src/encode.rs`、`src/decode.rs`、`src/codec_info.rs` の 3 か所にある。
- `.github/workflows/ci.yml` の GPU テストは `--test-threads=1` で直列化されており、この経路を検証していない。
- gdb で 8 並列の `Encoder::new` を観測すると、最大 5 スレッドが同時に `AMFFactoryHelper::Init` の中にいる。各スレッドの `Context` はすべて異なる。
- `CHANGES.md` の `2026.3.0` にある SIGSEGV 修正の `[FIX]` は、現コードでは成立していない。

## 設計方針

- `AmfLibrary::init_vulkan(&self, context: &Context, device)` を復活させ、`create_context` / `create_component` と同じ `self.inner.lock()` の下で `QueryInterface(AMFContext1)` と `InitVulkan` を実行する。
- `Context::init_vulkan` を `pub(crate)` にし、`AmfLibrary` 経由以外の呼び出しを塞ぐ。
- `src/encode.rs`、`src/decode.rs`、`src/codec_info.rs` の 3 か所を `AmfLibrary::instance().init_vulkan(&context, null)` に置き換える。
- ロック保持中に外部コードを呼ばない。この経路は FFI 呼び出しだけである。
- `.github/workflows/ci.yml` の `--test-threads=1` を外し、並列テストを実行する。外せない理由がある場合は CI にコメントで残す。
- 回帰テストを追加する。複数スレッドから `Encoder::new` を並列に呼ぶテスト、または crate 内のテストで複数の `Context` の `init_vulkan` を並列に呼ぶテストを用意する。`--test-threads=1` に依存しないテストにする。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 完了条件

- 並列初期化の回帰テストが SIGSEGV しない。
- CI が `--test-threads=1` なしで通る。
- `cargo test --workspace` を並列実行しても通る。
- `CHANGES.md` の `## develop` が更新されている。
- 過去に記録された直列化の設計と現コードの整合が取れている。
