# GPU デバイスの選択に対応する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/add-device-selection
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`AmfLibrary::create_context()` は AMF の既定の Vulkan デバイスで `Context` を初期化するため、複数 GPU を搭載した環境で使用する GPU を選べない。vpl-rs は `list_adapters()` と `AdapterSelector` を、nvcodec-rs は `device_count()` / `device_name()` / `device_id` を持っている。AMD GPU が iGPU と dGPU の 2 つある環境や、複数の dGPU を積むサーバーではデバイス選択が必要になる。

AMF を C から直接使う場合、利用者が作成した Vulkan デバイスを `AMFContext1::InitVulkan` に渡せば GPU を選べる。amf-rs でも同じことができるようにする。

## 現状

- `src/lib.rs` の `AmfLibrary::create_context` は `AMFFactory::CreateContext` で Context を作成したあと、`Context::init_vulkan(ptr::null_mut())` で既定の Vulkan デバイスを初期化する。デバイスの指定を受け取らない。
- `Encoder::new` / `Decoder::new` は利用者が作成した `Context` を受け取る。使用する GPU は `Context` の作成時に決まる。
- `src/amf.rs` の `Context::init_vulkan` は `pub(crate)` で `*mut std::ffi::c_void` を受け取り、そのポインタを `AMFContext1::InitVulkan` に渡している。
- `AMFContext1::GetVulkanDevice` と `GetVulkanDeviceExtensions` のラッパーはない。
- AMF にデバイスを列挙する API も、デバイスを番号で指定するプロパティもない。`AMFContext1::InitVulkan` は `AMFVulkanDevice` (`hInstance` / `hPhysicalDevice` / `hDevice`) へのポインタを受け取る。
- `AMFVulkanDevice` は `core/VulkanAMF.h` に定義されている。build.rs の bindgen はこのヘッダーを読み込んでいないため、`ffi` からも使えない。
- AMF のサンプルは Vulkan のインスタンスを作成して物理デバイスを列挙し、拡張と feature を有効化した `VkDevice` を作って `InitVulkan` に渡している。

## 設計方針

案 A を採用する。利用者が Vulkan デバイスを作成し、`Context` の作成時に渡す。

- 案 B (crate 内でインスタンス作成と物理デバイス列挙を行う) は採用しない。インスタンス作成、キューファミリ選択、AMF が要求する拡張と feature の有効化、破棄順序まで crate が抱えることになり、AMF の C API の薄いバインディングという責務を越えるため。
- 案 C (低レベル API だけを追加する) は案 A に含める。
- デバイスは `Context` が表すため、`EncoderConfig` / `DecoderConfig` には持たせない。同じ `Context` を共有する `Encoder` と `Decoder` が別のデバイスを指定できると矛盾するため。

追加する API:

```rust
// src/amf.rs
/// AMF に渡す Vulkan デバイスのハンドル一式 (AMFVulkanDevice に対応)
#[derive(Debug, Clone, Copy)]
pub struct VulkanDevice { /* instance, physical_device, device */ }

impl VulkanDevice {
    /// # Safety
    /// 3 つとも有効な Vulkan ハンドルを指すこと。
    pub unsafe fn from_raw(
        instance: *mut std::ffi::c_void,
        physical_device: *mut std::ffi::c_void,
        device: *mut std::ffi::c_void,
    ) -> Self;

    pub fn instance(&self) -> *mut std::ffi::c_void;
    pub fn physical_device(&self) -> *mut std::ffi::c_void;
    pub fn device(&self) -> *mut std::ffi::c_void;
}

impl Context {
    /// AMF が使っている Vulkan デバイスを返す
    pub fn vulkan_device(&self) -> Result<VulkanDevice, Error>;
}

// src/lib.rs
impl AmfLibrary {
    /// 既定の Vulkan デバイスで Context を作成する (現状維持)
    pub fn create_context(&self) -> Result<Context, Error>;

    /// 利用者が作成した Vulkan デバイスで Context を作成する
    ///
    /// # Safety
    /// device のハンドルが、返した Context とその Clone、およびそこから
    /// 作成した Encoder / Decoder / Surface / Buffer / Component をすべて
    /// 破棄するまで有効であること。
    pub unsafe fn create_context_with_vulkan_device(
        &self,
        device: amf::VulkanDevice,
    ) -> Result<Context, Error>;

    /// AMF が Vulkan 処理に使うデバイス拡張の一覧
    pub fn vulkan_device_extensions(&self) -> Result<Vec<String>, Error>;
}
```

- `AMFVulkanDevice` は手書きで写さない。`core/VulkanAMF.h` を bindgen に読み込ませて生成する。Vulkan のハンドル型だけを宣言したスタブヘッダーを build.rs で生成して `-I` で渡し、実 Vulkan ヘッダーへのビルド依存は増やさない。AMF のヘッダーが変われば生成物が変わり、コンパイルエラーになる。docs.rs 用のダミーバインディングにも `AMFVulkanDevice` を追加する。
- `AMFVulkanDevice` の `cbSizeof` には生成した構造体のサイズを、`pNext` には null を設定する。`create_context_with_vulkan_device` はハンドルが null の場合に `InvalidArgument` を返す。
- `create_context_with_vulkan_device` は unsafe とし、デバイスのハンドルが `Context` と派生ハンドルより長生きすることを Safety に書く。`Context::vulkan_device` が返すハンドルは `Context` が生きている間だけ有効であることも doc に書く。
- `vulkan_device_extensions` は `InitVulkan` の前、つまりデバイスを作る前に必要になる。AMF のサンプルは未初期化の Context に対して `GetVulkanDeviceExtensions` を呼んでいるが、API リファレンスには未初期化の場合に `AMF_NO_DEVICE` と書かれている。実機で確認し、失敗する場合は既定デバイスで初期化した一時的な Context から取得する。
- `InitVulkan` に渡した `AMFVulkanDevice` を呼び出し後に解放してよいか (AMF がハンドルをコピーするか) を実機で確認する。AMF がポインタを保持する場合は `Context` に構造体を保持させる。
- AMF が要求するデバイス拡張は `vulkan_device_extensions` で取得できる。インスタンス拡張、feature、キューファミリは AMF が公開していないため、doc では AMF のサンプルの手順 (インスタンス作成、物理デバイス選択、拡張の有効化、コンピュートキューの作成、feature の有効化) を示す。AMF はコンピュート対応キューファミリのインデックス 0 のキューを使う。
- 複数 GPU を使う場合は GPU ごとに `Context` を作る。Context をまたいだ Surface / Buffer の受け渡しは既存の契約どおりエラーになる。
- 指定したデバイスを AMF が受け付けない場合、`InitVulkan` は `AMF_NOT_SUPPORTED` などのステータスを返す。呼び出し側が `AmfError::status()` で判別できることを doc に書く。
- 実機テストは、既定デバイスで従来どおり動作すること、既定で作成した `Context` の `vulkan_device()` から `VulkanDevice` を作って別の `Context` を作成しエンコードできること (元の `Context` は後の `Context` より後に破棄する)、null ハンドルが `InvalidArgument` になること、`vulkan_device_extensions` が呼べることを確認する。Vulkan デバイス作成用の依存は追加しない。

## 完了条件

- `AmfLibrary::create_context_with_vulkan_device` で利用者が作成した Vulkan デバイスを指定でき、未指定時は既定デバイスを使う。
- `AmfLibrary::vulkan_device_extensions` で AMF が要求するデバイス拡張を取得できる。
- `Context::vulkan_device` で AMF が使っている Vulkan デバイスを取得できる。
- `AMFVulkanDevice` が bindgen で生成されており、手書きのレイアウト定義がない。
- `VkDevice` の所有権と寿命、生ポインタを扱う API の Safety、拡張・feature・キューの要件が doc に書かれている。
- 実機で既定デバイスと指定デバイスの両方の動作を確認するテストが追加される。
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が通る。
- README と `skills/shiguredo-amf/SKILL.md` に使い方を追加する。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。
