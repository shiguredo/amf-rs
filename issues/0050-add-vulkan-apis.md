# AMF の Vulkan 連携 API を一通り利用できるようにする

- Created: 2026-11-22
- Completed: {YYYY-MM-DD}
- Branch: feature/add-vulkan-apis
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

amf-rs は AMF の C API の薄いバインディングであり、C から直接使える操作は Rust からも使えることを目標とする。Vulkan 連携については、次の操作がまだできない。

- AMF が作成・使用する Vulkan デバイスをアプリ側から使うときの排他 (`LockVulkan` / `UnlockVulkan`)
- アプリが持つ `VkImage` / `VkBuffer` を AMF の `Surface` / `Buffer` として包む (`CreateSurfaceFromVulkanNative` / `CreateBufferFromVulkanNative`)
- usage と CPU access を指定したサーフェス・バッファの確保 (`AllocSurfaceEx` / `AllocBufferEx`)
- `InitVulkan` の前のプロパティ設定 (`AMF_CONTEXT_VULKAN_COMPUTE_QUEUE` / `AMF_CONTEXT_VULKAN_USE_TIMELINE_SEMAPHORES`)

AMF のサンプルはスワップチェーンのバックバッファを `CreateSurfaceFromVulkanNative` で包み、共有デバイスへのアクセスを `AMFContext1::AMFVulkanLocker` で囲み、`InitVulkan` の前にコンピュートキューと timeline semaphore を設定している。Rust 経由でも同じパイプラインを組めるようにする。

## 現状

- `src/amf.rs` の `Context` は `as_ptr` で生ポインタを返す。`AMFContext1` は `Context::init_vulkan` の内部で `QueryInterface` しているが、`LockVulkan` / `UnlockVulkan` を呼ぶ API はない。
- `CreateSurfaceFromVulkanNative` / `CreateBufferFromVulkanNative` を呼ぶ API はない。`Surface::from_raw` / `Buffer::from_raw` は公開されており、AMF が返したポインタからハンドルを作ることはできる。
- `Context::alloc_surface` は `AMFContext::AllocSurface` を、`Context::alloc_buffer` は `AllocBuffer` を呼ぶ。usage と CPU access の引数はない。
- `AmfLibrary::create_context` は `AMFFactory::CreateContext` の直後に `Context::init_vulkan` を呼ぶ。間にプロパティを設定する経路がない。
- `Context` はプロパティを設定する API を持たない。`PropertyStorage` は `Surface` / `Buffer` / `Component` / `Data` から取得できるが、`Context` からは取得できない。`PropertyStorage` には bool を設定するメソッドもない。
- `AMFVulkanSurface` / `AMFVulkanBuffer` / `AMFVulkanSync` / `AMFVulkanView`、`AMF_CONTEXT_VULKAN_COMPUTE_QUEUE`、`AMF_CONTEXT_VULKAN_USE_TIMELINE_SEMAPHORES` は `core/VulkanAMF.h` に定義されている。ビルド時の bindgen はこのヘッダーを読み込んでいないため、これらの型と定数は `ffi` からも使えない。
- `AMF_SURFACE_USAGE_BITS` と `AMF_BUFFER_USAGE_BITS`、`AMF_MEMORY_CPU_ACCESS` は `core/Surface.h` と `core/Buffer.h` に定義されており、bindgen の生成物には含まれている。
- Vulkan デバイスの選択と `GetVulkanDevice` は 0047 で扱う。

## 設計方針

- `core/VulkanAMF.h` を bindgen に読み込ませ、`AMFVulkanDevice` などの型と Vulkan 関連の定数を生成する。Vulkan のハンドル型は build.rs で生成する最小のスタブヘッダーで宣言し、実 Vulkan ヘッダーへのビルド依存は増やさない。
- `LockVulkan` / `UnlockVulkan` は RAII のガード型として公開する (例: `Context::lock_vulkan() -> Result<VulkanLock<'_>, Error>`)。`Drop` で `UnlockVulkan` を呼び、解放漏れを防ぐ。
- ネイティブリソースは、公開 API に `ffi` の型を出さず、`VkImage` / `VkBuffer` と同期情報を表す専用の型を置く。AMF がネイティブリソースを破棄するのか利用者が破棄するのか、`AMFVulkanSync` のセマフォとフェンスをどちらが待つのかを確認し、`# Safety` と doc に書く。
- `AMF_SURFACE_USAGE` と `AMF_BUFFER_USAGE` はビットフラグなので、複数のフラグを組み合わせられる Rust の型を用意する。`AMF_SURFACE_USAGE_DEFAULT` と `AMF_SURFACE_USAGE_NONE` の意味、指定できる組み合わせ、メモリ種別との整合性を doc に書く。利用者が渡す値なので、未知のビットと矛盾する組み合わせを検証する。
- `InitVulkan` 前のプロパティ設定は、初期化前の Context を表す型を用意する案と、作成 API に設定値を持たせる案を比較する。`Encoder::new` / `Decoder::new` は初期化済みの `Context` だけを受け取る現在の契約を保つ。
- `AMF_CONTEXT_VULKAN_COMPUTE_QUEUE` の値は、利用者が作成した Vulkan デバイスのキューファミリの範囲と一致している必要がある。amf-rs は Vulkan のキューファミリを列挙しないため値の検証はできない。この制約を doc に書く。
- ハンドルの寿命の契約がある API は `unsafe` とし、`# Safety` に書く。
- 実機テストでは、ロックを伴うネイティブサーフェスの利用、外部 `VkImage` を AMF に包ませたエンコード、usage と CPU access を指定した確保、コンピュートキューと timeline semaphore を指定した初期化を確認する。

## 完了条件

- `Context` から Vulkan デバイスのロックとアンロックができる。
- 外部の `VkImage` / `VkBuffer` を AMF の `Surface` / `Buffer` として扱える。
- usage と CPU access を指定してサーフェスとバッファを確保でき、未指定時は従来と同じ結果になる。
- `InitVulkan` の前に `AMF_CONTEXT_VULKAN_COMPUTE_QUEUE` と `AMF_CONTEXT_VULKAN_USE_TIMELINE_SEMAPHORES` を設定できる。
- 上記の経路が実機で通り、doc に所有権・寿命・同期・値の制約が書かれている。
- README と `skills/shiguredo-amf/SKILL.md` に使い方を追加する。
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
