# GPU デバイスの選択に対応する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-device-selection
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

現在の `Encoder` / `Decoder` は `Context::init_vulkan(ptr::null_mut())` で AMF の既定デバイスを使うため、複数 GPU を搭載した環境で使用する GPU を選べない。vpl-rs は `list_adapters()` と `AdapterSelector` を、nvcodec-rs は `device_count()` / `device_name()` / `device_id` を持っている。AMD GPU が iGPU と dGPU の 2 つある環境や、複数の dGPU を積むサーバーではデバイス選択が必要になる。

## 現状

- `src/lib.rs` の `AmfLibrary::create_context` は `AMFFactory::CreateContext` を呼ぶだけで、デバイスの指定を受け取らない。
- `src/encode.rs` の `Encoder::new` と `src/decode.rs` の `Decoder::new` は `unsafe { context.init_vulkan(ptr::null_mut()) }` で既定デバイスを使う。
- `src/amf.rs` の `Context::init_vulkan(device)` は `*mut c_void` を受け取るが、公開 API からは呼ばれていない。
- AMF の `AMFContext1::InitVulkan(void* pVulkanDevice)` は `VkDevice` を受け取る。AMF 自身にデバイスを列挙する API はない。
- `AMFContext1::GetVulkanDevice()` で AMF が作成した `VkDevice` を取得できるが、`ContextFuncs` にラッパーがない。

## 設計方針

複数の案を比較して方式を決める。

- 案 A: 利用者が作成した `VkDevice` を `EncoderConfig` / `DecoderConfig` または `AmfLibrary` に渡す。Vulkan の初期化と列挙は利用者の責務にする。`ash` などの依存を追加しない。
- 案 B: crate 内で Vulkan デバイスを列挙する。`ash` などの依存を追加し、`list_devices()` とデバイス ID を提供する。vpl-rs / nvcodec-rs に近い UX になるが、依存とプラットフォーム対応が増える。
- 案 C: `AmfLibrary::create_context_with_vulkan_device(device)` のような低レベル API だけを追加し、高レベルの選択 API は別 issue にする。

どの案でも以下を決める。

- `VkDevice` の所有権と寿命を明記する。AMF が保持するのか、利用者が `Init` の間だけ有効にすればよいのかを確認する。
- 生ポインタを公開 API に出す場合の安全性の扱いを決める。`unsafe` を強制するか、所有権を持つラッパー型を用意する。
- `GetVulkanDevice()` を公開し、AMF が選んだデバイスを確認できるようにする。
- デバイスを指定しない場合は、現状どおり既定デバイスを使う。
- 複数 GPU 環境での選択と、指定したデバイスが AMF に拒否された場合のエラーを doc に書く。

実機テストは、既定デバイスで従来どおり動作することと、外部から渡した `VkDevice` で `Init` できることを検証する。テスト用に Vulkan デバイスを作るコードが必要になるため、追加の依存が許容できるかを判断する。

README と `skills/shiguredo-amf/SKILL.md` に使い方を追加する。

## 完了条件

- 採用する案 (A / B / C) とその理由が決まり、issue 本文または doc に書かれている。
- 利用者が使用する GPU を指定でき、未指定時は既定デバイスを使う。
- `VkDevice` の所有権と寿命、生ポインタを扱う場合の安全性が doc に書かれている。
- 実機で既定デバイスと指定デバイスの両方の動作を確認するテストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
