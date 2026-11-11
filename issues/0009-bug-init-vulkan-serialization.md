# `AmfLibrary` と `Context` の呼び出しを直列化し、並列初期化時の SIGSEGV を修正する

- Created: 2026-11-10
- Completed: 2026-11-11
- Branch: feature/fix-init-vulkan-serialization
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Context::init_vulkan` が `AmfLibrary` の Mutex を取らずに `AMFContext1::InitVulkan` を呼んでいる。`InitVulkan` の内部 (`AMFDeviceImpl<AMFDeviceVulkan>` の構築や `CreateCommandPool`) は `AMFFactoryHelper::Init` を呼び、この関数は非 thread-safe な遅延初期化を行う。ハンドルを非アトミックに確認し、`dlopen` とフィールドの書き込みを行うため、複数スレッドが同時に呼ぶと SIGSEGV する。

別々の `Context` に対して並列に `init_vulkan` を呼ぶだけで SIGSEGV することを実機で確認済みである (64 個の `Context` に対する並列 `init_vulkan` が 5 回中 5 回 SIGSEGV)。`AMFInit` は静的 factory を返すだけで `AMFFactoryHelper::Init` を呼ばないため、`AmfLibrary` と `Context` の呼び出しを同じ Mutex で直列化する必要がある。

過去に SIGSEGV の修正として「`AMFInit` と `InitVulkan` を同一の Mutex で直列化する」設計が記録されているが、AMF オブジェクトの RAII ラッパー化によってそのロックが失われた回帰である。

## 修正前の状態

- `src/amf.rs` の `Context::init_vulkan` は `AmfLibrary` の Mutex を取らない。
- `src/lib.rs` の `AmfLibrary` の Mutex は `query_version`、`create_context`、`create_component` でのみ取得される。
- `Context::init_vulkan` の呼び出しは `src/encode.rs`、`src/decode.rs`、`src/codec_info.rs` の 3 か所にある。
- `.github/workflows/ci.yml` の GPU テストは `--test-threads=1` で直列化されており、この経路を検証していない。
- gdb で 8 並列の `Encoder::new` を観測すると、最大 5 スレッドが同時に `AMFFactoryHelper::Init` の中にいる。各スレッドの `Context` はすべて異なる。
- `CHANGES.md` の `2026.3.0` にある SIGSEGV 修正の `[FIX]` は、現コードでは成立していない。

## 調査結果

### 再現

- 実機の `libamfrt64.so.1.4.37` で、64 個の `Context` を事前に作成し、それぞれに対して並列に `AMFContext1::InitVulkan(NULL)` を呼ぶと、修正前は 5 回中 5 回 SIGSEGV した。
- gdb のバックトレースでは `AMFFactoryHelper::Init` が最上位に現れた。
- `AMFDeviceImpl<AMFDeviceVulkan>` のコンストラクタは `AMFFactoryHelper::Init` と `Terminate` をペアで呼ぶ。

### `AMFFactoryHelper` の状態

- `libamfrt64.so.1` は `g_AMFFactory` (C++ の `AMFFactoryHelper`) をエクスポートしている。
- `AMFFactoryHelper::Init` / `Terminate` は `m_hDLLHandle` と参照カウントを非アトミックに確認・更新し、ロックを取らない。
- `Init` はハンドルが `NULL` のとき `dlopen` と `AMFInit` を行い、`Terminate` は参照カウントが 0 になると `dlclose` する。
- `AMFInit` は静的な `AMFFactory*` を返すだけで、`AMFFactoryHelper::Init` を呼ばない。
- 独立した C プローブで、`AMFInit`、`CreateContext`、`InitVulkan`、`AllocSurface`、`CreateComponent`、`Component::Init` の各後も、`g_AMFFactory` のハンドルが `NULL`、参照カウントが 0 のままであることを確認した。Context / Surface / Component が生存していても、helper は保持されない。
- つまり `AMFFactoryHelper` は常駐する初期化オブジェクトではなく、各操作の間だけ `Init` / `Terminate` される一時的な loader である。

### 呼び出し元

- `AMFFactoryHelper::Init` の呼び出し元はランタイム内に多数ある。
- 確認できた主な経路:
  - `AMFDeviceImpl<AMFDeviceVulkan>` のコンストラクタ (InitVulkan)
  - `AMFDeviceVulkanImpl::CreateCommandPool`
  - `AMFDeviceHostImpl::CreateSurface` / `CreateBuffer`
  - `AMFEncoderCoreH264Impl::Init` / `AMFDecoderUVDImpl::Init`
  - `AMFTraceW` / `AMFDebugImpl`
  - `AMFGPUEventProfiler` の内部スレッド
- したがって、クライアントが `AMFFactoryHelper` を使わなくても、`InitVulkan` や `Component::Init` などの公開 API の内部で helper が使われる。

### C API と C++ API の差

- C API は `AMFInit` と `AMFFactory` の vtable を公開するが、`AMFFactoryHelper` 相当の C API はエクスポートしていない。
- C++ ヘッダ `amf/public/common/AMFFactory.h` には `extern ::AMFFactoryHelper g_AMFFactory;` があり、C++ からはランタイムの `g_AMFFactory` を参照できる。ただしこれは公開 ABI として保証されたものではなく、実装詳細のシンボルに依存する。
- 公式 C++ サンプルは `g_AMFFactory.Init()` を起動時に呼ぶが、`AMFFactory.cpp` をサンプル自身にコンパイルしている。これはクライアント側の helper であり、ランタイム内部の `g_AMFFactory` を初期化するものではない。
- C サンプルも `AMFFactoryC.c` で同等のクライアント側 helper を持つ。
- したがって「C++ ならランタイムの helper を保持できる」ではなく、C++ でも公開 API としては保持できない。C++ の型定義があるため、`extern` / `dlsym` で内部シンボルを直接叩けるだけである。

### スレッド安全性のドキュメント

- `AMFComponent` は thread-safe と明記されている。
- `AMFPropertyStorage` / `AMFData` / `AMFSurface` は thread-safe ではない。
- `AMFContext` / `AMFPlane` には thread-safety の記述がない。
- `AMFContextImpl::InitVulkan` は Context ごとの `AMFLock` を取るが、`AMFFactoryHelper` はプロセス全体で共有されるため、別 Context 同士の排他にはならない。
- Wiki には「AMF framework leaves threading to the application」とあるが、`AMFFactoryHelper` の初期化をアプリ側でどう直列化するかは書かれていない。
- AMF のドキュメント不足は既知で、AMD の担当者もドキュメント改善が必要とコメントしている。

## AMF 側の問題点

1. プロセス全体の `AMFFactoryHelper` が非 thread-safe なのに、ランタイム内部の通常経路から呼ばれる。
2. 公開 C API に helper を明示的に初期化・保持する手段がない。
3. C++ の `AMFFactoryHelper` はクライアント側 helper で、ランタイム内部の helper を pin できない。ランタイムの `g_AMFFactory` を直接使うのは内部 ABI 依存。
4. 並列 Context 初期化がサポート外であることがドキュメントに明記されていない。
5. AMF 内部スレッド (`AMFGPUEventProfiler` など) も helper を使うため、アプリ側の Mutex だけでは完全に排他できない。
6. helper 以外にも `s_pTrace` / `s_pDebug`、cache folder、programs、performance monitor、Vulkan import table などの共有状態がある。

## 設計方針

- `AmfLibrary` と `Context` の操作を同じ `AmfObjectState` のロックで直列化する。`Context` では `init_vulkan` / `alloc_surface` / `alloc_buffer` / `terminate` を対象とする。
- `AmfLibrary` が作成する `Context` は、`AmfLibrary` が保持する `AmfObjectState` を共有する。
- `Context::init_vulkan` は `Context` のメソッドのまま `pub(crate)` にし、`AmfLibrary::create_context` で作成した Context に対して呼ぶ。
- `AmfLibrary::init_vulkan` は設けない。`AmfLibrary` はランタイムのライフサイクルと factory の操作に限定する。
- `src/encode.rs`、`src/decode.rs`、`src/codec_info.rs` の 3 か所は `Context::init_vulkan` を呼ぶ。
- `.github/workflows/ci.yml` の `--test-threads=1` を外し、並列テストを実行する。外せない理由がある場合は CI にコメントで残す。
- 回帰テストを追加する。複数スレッドから `Encoder::new` を並列に呼ぶテスト、または crate 内のテストで複数の `Context` の `init_vulkan` を並列に呼ぶテストを用意する。`--test-threads=1` に依存しないテストにする。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 完了条件

- 並列初期化の回帰テストが SIGSEGV しない。
- CI が `--test-threads=1` なしで通る。
- `cargo test --workspace` を並列実行しても通る。
- `CHANGES.md` の `## develop` が更新されている。
- `AmfLibrary` / `Context` のロックと現コードの整合が取れている。

## 解決方法

- `AmfLibrary` に `AmfObjectState` を持たせ、`create_context` で作成する `Context` に同じ状態を共有させた。これにより `AmfLibrary` の factory 操作と `Context` の操作が同じロックで直列化される。
- `Context` の `init_vulkan` / `alloc_surface` / `alloc_buffer` / `terminate` を共有ロックの対象にした。
- `Context::init_vulkan` は `Context` のメソッドのまま `pub(crate)` にし、`AmfLibrary::init_vulkan` は設けていない。
- 64 個の `Context` を並列に `init_vulkan` する回帰テストを追加した。修正前は 5 回中 5 回 SIGSEGV し、修正後は 5 回連続で成功した。
- GPU CI から `--test-threads=1` を外し、`CHANGES.md` と `skills/shiguredo-amf/SKILL.md` を更新した。

## 残存リスク

- `Component::init` は `AmfLibrary` / `Context` の共有ロックに含めていない。`AMFComponent` は thread-safe と明記されているが、`Init` が内部で `AMFFactoryHelper` を触るため、並列 `Encoder::new` で helper 競合が残る可能性がある。
- AMF の内部スレッド (`AMFGPUEventProfiler` など) はアプリ側のロックでは排他できない。
- `AMFFactoryHelper` 以外にも `s_pTrace` / `s_pDebug`、cache folder、programs、performance monitor、Vulkan import table などの共有状態がある。公開 API の全経路を安全にするには、AMD 側の修正か、内部 helper を pin する C++ shim が必要になる。
