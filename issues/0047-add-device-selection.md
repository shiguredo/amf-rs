# GPU デバイスの選択に対応する

- Created: 2026-11-10
- Completed: 2026-11-22
- Branch: feature/add-device-selection
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`AmfLibrary::create_context()` は AMF の既定の Vulkan デバイスで `Context` を初期化するため、複数 GPU を搭載した環境で使用する GPU を選べない。vpl-rs は `list_adapters()` と `AdapterSelector` を、nvcodec-rs は `device_count()` / `device_name()` / `device_id` を持っている。AMD GPU が iGPU と dGPU の 2 つある環境や、複数の dGPU を積むサーバーではデバイス選択が必要になる。

AMF を C から直接使う場合、利用者が作成した Vulkan デバイスを `AMFContext1::InitVulkan` に渡せば GPU を選べる。amf-rs でも同じことができるようにする。

## 現状

issue 作成時点の状態を記載する。

- `src/lib.rs` の `AmfLibrary::create_context` は `AMFFactory::CreateContext` で Context を作成したあと、`Context::init_vulkan(ptr::null_mut())` で既定の Vulkan デバイスを初期化する。デバイスの指定を受け取らない。
- `Encoder::new` / `Decoder::new` は利用者が作成した `Context` を受け取る。使用する GPU は `Context` の作成時に決まる。
- `src/amf.rs` の `Context::init_vulkan` は `pub(crate)` で `*mut std::ffi::c_void` を受け取り、そのポインタを `AMFContext1::InitVulkan` に渡している。
- `AMFContext1::GetVulkanDevice` と `GetVulkanDeviceExtensions` のラッパーはない。
- AMF にデバイスを列挙する API も、デバイスを番号で指定するプロパティもない。`AMFContext1::InitVulkan` は `AMFVulkanDevice` (`hInstance` / `hPhysicalDevice` / `hDevice`) へのポインタを受け取る。
- `AMFVulkanDevice` は `core/VulkanAMF.h` に定義されている。build.rs の bindgen はこのヘッダーを読み込んでいないため、`ffi` からも使えない。
- AMF のサンプルは Vulkan のインスタンスを作成して物理デバイスを列挙し、拡張と feature を有効化した `VkDevice` を作って `InitVulkan` に渡している。

## 設計方針

案 A を採用する。利用者が Vulkan デバイスを作成し、`Context` の初期化時に渡す。

- 案 B (crate 内でインスタンス作成と物理デバイス列挙を行う) は採用しない。インスタンス作成、キューファミリ選択、AMF が要求する拡張と feature の有効化、破棄順序まで crate が抱えることになり、AMF の C API の薄いバインディングという責務を越えるため。
- 案 C (低レベル API だけを追加する) は案 A に含める。
- デバイスは `Context` が表すため、`EncoderConfig` / `DecoderConfig` には持たせない。同じ `Context` を共有する `Encoder` と `Decoder` が別のデバイスを指定できると矛盾するため。

追加する API:

```rust
// src/amf.rs
/// AMF に渡す Vulkan デバイスのハンドル一式 (AMFVulkanDevice に対応)
#[derive(Debug, Clone, Copy)]
pub struct VulkanDevice { /* AMFVulkanDevice */ }

impl VulkanDevice {
    /// # Safety
    /// `null` 以外のハンドルは有効な Vulkan ハンドルを指すこと。
    pub unsafe fn from_raw(
        instance: *mut std::ffi::c_void,
        physical_device: *mut std::ffi::c_void,
        device: *mut std::ffi::c_void,
    ) -> Self;

    pub fn instance(&self) -> *mut std::ffi::c_void;
    pub fn physical_device(&self) -> *mut std::ffi::c_void;
    pub fn device(&self) -> *mut std::ffi::c_void;
}

/// Vulkan 未初期化の AMFContext を保持するハンドル
pub struct UninitializedContext { /* ... */ }

impl UninitializedContext {
    /// AMF が Vulkan 処理に使うデバイス拡張の一覧
    pub fn vulkan_device_extensions(&self) -> Result<Vec<String>, Error>;

    /// 既定の Vulkan デバイスで初期化し、Context を返す
    pub fn init_vulkan(self) -> Result<Context, Error>;

    /// 指定した Vulkan デバイスで初期化し、Context を返す
    ///
    /// # Safety
    /// device のハンドルが、返した Context とその Clone、およびそれらから
    /// 取得したすべての AMF オブジェクトを破棄するまで有効であること。
    pub unsafe fn init_vulkan_with_device(
        self,
        device: VulkanDevice,
    ) -> Result<Context, Error>;
}

/// Vulkan 初期化済みの Context
impl Context {
    /// 初期化に使用した Vulkan デバイスを返す
    pub fn vulkan_device(&self) -> Result<VulkanDevice, Error>;
}

// src/lib.rs
impl AmfLibrary {
    /// 既定の Vulkan デバイスで初期化した Context を作成する
    pub fn create_context(&self) -> Result<Context, Error>;

    /// Vulkan 未初期化の AMFContext を保持する UninitializedContext を作成する
    pub fn create_uninitialized_context(&self) -> Result<UninitializedContext, Error>;
}
```

- `create_context` は現状どおり既定の Vulkan デバイスで初期化した `Context` を返す。一般的な使い方を変えない。
- GPU を選ぶ場合は `create_uninitialized_context` で `UninitializedContext` を作成する。同じ `UninitializedContext` で `vulkan_device_extensions` を呼び、Vulkan デバイスを作成してから `init_vulkan_with_device` で初期化して `Context` を得る。これは AMF のサンプルと同じく、後で初期化する同じ `Context` に対して拡張を取得する形になる。
- `UninitializedContext` は初期化で消費する。`Context` は常に初期化済みなので、`Encoder::new` / `Decoder::new` は `Context` だけを受け取る。初期化状態は型で表す。
- `AMFVulkanDevice` は手書きで写さない。`core/VulkanAMF.h` を bindgen に読み込ませて生成する。Vulkan のハンドル型だけを宣言したスタブヘッダーを build.rs で生成して `-I` で渡し、実 Vulkan ヘッダーへのビルド依存は増やさない。AMF のヘッダーが変われば生成物が変わり、コンパイルエラーになる。docs.rs 用のダミーバインディングにも `AMFVulkanDevice` を追加する。
- `VulkanDevice::from_raw` は `AMFVulkanDevice` の `cbSizeof` に生成した構造体のサイズ、`pNext` に null を設定する。`init_vulkan_with_device` はハンドルが null の場合に `InvalidArgument` を返す。
- `init_vulkan_with_device` は unsafe とし、デバイスのハンドルが `Context` とそこから取得したすべての AMF オブジェクトより長生きすることを Safety に書く。`Context::vulkan_device` が返すハンドルは `Context` が生きている間だけ有効であることも doc に書く。
- `InitVulkan` に渡した `AMFVulkanDevice` を呼び出し後に解放してよいか (AMF がハンドルをコピーするか) を実機で確認する。AMF がポインタを保持する場合は `Context` に構造体を保持させる。
- AMF が要求するデバイス拡張は `UninitializedContext::vulkan_device_extensions` で取得できる。インスタンス拡張、feature、キューファミリは AMF が公開していないため、doc では AMF のサンプルの手順 (インスタンス作成、物理デバイス選択、拡張の有効化、コンピュートキューの作成、feature の有効化) を示す。AMF はコンピュート対応キューファミリのインデックス 0 のキューを使う。
- 複数 GPU を使う場合は GPU ごとに `Context` を作る。Context をまたいだ Surface / Buffer の受け渡しは既存の契約どおりエラーになる。
- 指定したデバイスを AMF が受け付けない場合、`InitVulkan` は `AMF_NOT_SUPPORTED` などのステータスを返す。呼び出し側が `AmfError::status()` で判別できることを doc に書く。
- 実機テストは、既定デバイスで動作すること、未初期化の `UninitializedContext` から `vulkan_device_extensions` が呼べること、既定で作成した `Context` の `vulkan_device()` から `VulkanDevice` を作って別の `Context` を初期化しエンコードできること (元の `Context` は後の `Context` より後に破棄する)、null ハンドルが `InvalidArgument` になることを確認する。Vulkan デバイス作成用の依存は追加しない。

## 完了条件

- `AmfLibrary::create_context` が既定の Vulkan デバイスで初期化した `Context` を返す。
- `AmfLibrary::create_uninitialized_context` で Vulkan 未初期化の `UninitializedContext` を作成できる。
- `UninitializedContext::vulkan_device_extensions` で AMF が要求するデバイス拡張を、Vulkan デバイスを作成する前に取得できる。
- `UninitializedContext::init_vulkan` / `init_vulkan_with_device` で `Context` を初期化できる。
- `Context::vulkan_device` で AMF が使っている Vulkan デバイスを取得できる。
- `Context` は常に初期化済みで、`Encoder::new` / `Decoder::new` は `Context` だけを受け取る。
- `AMFVulkanDevice` が bindgen で生成されており、手書きのレイアウト定義がない。
- `VkDevice` の所有権と寿命、生ポインタを扱う API の Safety、拡張・feature・キューの要件が doc に書かれている。
- 実機で既定デバイスと指定デバイスの両方の動作を確認するテストが追加される。
- `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が通る。
- README と `skills/shiguredo-amf/SKILL.md` に使い方を追加する。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。

## 解決方法

- `amf::VulkanDevice` を追加した。`unsafe VulkanDevice::from_raw(instance, physical_device, device)` で 3 つの Vulkan ハンドルを保持し、`instance()` / `physical_device()` / `device()` で取得できる。
- `amf::UninitializedContext` を追加した。Vulkan 未初期化の `AMFContext` を保持し、`vulkan_device_extensions()` で拡張を取得できる。`init_vulkan()` または `init_vulkan_with_device()` は `self` を消費して `Context` を返す。
- `AmfLibrary::create_uninitialized_context` を追加した。既存の `create_context` は `create_uninitialized_context()?.init_vulkan()` として実装し、既定デバイスで初期化した `Context` を返す挙動を維持した。
- `UninitializedContext::init_vulkan_with_device` は null ハンドルを `InvalidArgument` で拒否する。AMF が渡した `AMFVulkanDevice` へのポインタを保持することを実機で確認したため、`Arc<VulkanDevice>` を `UninitializedContext` から `Context` へ移動し、最後の `Context` が破棄されるまで保持する。初期化に失敗した場合も、AMFContext の破棄後に `Arc` が破棄されるよう `UninitializedContext` のフィールド順を保っている。
- `Context` は常に Vulkan 初期化済みの型にした。初期化状態を実行時に持たないため、`Encoder::new` / `Decoder::new` は型で初期化済みを保証できる。
- `Context::vulkan_device` を追加した。指定したデバイスは初期化時に渡したハンドルを返し、既定のデバイスは `AMFContext1::GetVulkanDevice` が返す `AMFVulkanDevice` から取得する。
- `build.rs` で Vulkan のハンドル型だけを宣言した `vulkan/vulkan.h` を生成して `-I` に追加し、`core/VulkanAMF.h` を bindgen に読み込ませた。`AMFVulkanDevice` は生成された定義を使い、手書きのレイアウト定義は持たない。docs.rs 用のダミーバインディングにも `AMFVulkanDevice` を追加した。
- doc に `VkDevice` の所有権と寿命、`VulkanDevice` と `init_vulkan_with_device` の Safety、デバイス拡張とキューの要件、`AmfError::status` でのステータス判別を書いた。
- `tests/test_amf.rs` に、未初期化の `UninitializedContext` から `vulkan_device_extensions` が呼べること、`init_vulkan` と `init_vulkan_with_device` で初期化できること、既定のデバイスから取得した `VulkanDevice` で別の `Context` を初期化してエンコードできること、取得元の `Context` を後に破棄する順序、null ハンドルが `InvalidArgument` になることを追加した。
- README / `skills/shiguredo-amf/SKILL.md` / `CHANGES.md` を更新した。
- AMD GPU 実機で `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` を通した。
