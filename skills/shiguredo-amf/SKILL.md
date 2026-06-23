---
name: shiguredo-amf
description: 時雨堂の AMD AMF (Advanced Media Framework) Rust バインディング shiguredo_amf の機能・API リファレンス。H.264 / H.265 / AV1 のハードウェアエンコード・デコード、AMF ランタイムの動的ロード、Surface / Buffer 管理、ハンドラーベースのコールバック設計、エンコード中の動的プロパティ再設定に関する質問時に使用。
---

# shiguredo_amf

AMD GPU 向けハードウェアアクセラレーションによるビデオエンコード/デコードを提供する Rust バインディング。

## 特徴

- **AMD AMF (Advanced Media Framework)** ベースの H.264 / H.265 / AV1 ハードウェアエンコード・デコード
- **AMF ランタイムライブラリ** (`libamfrt64.so.1`) を `dlopen` で動的にロード (ビルド時のリンク不要)
- **Vulkan バックエンド**で GPU 処理を実行 (Linux x86_64)
- **エンコード入力フォーマット選択**: NV12 / YV12 / I420 / BGRA / ARGB / RGBA / YUY2 / UYVY / P010 / P012 / P016 / Y210 / AYUV / Y410 / Y416
- **デコード出力**は NV12 フォーマット
- **フレーム単位のエンコードオプション** (IDR フレーム強制)
- **エンコード中の動的プロパティ再設定** (`Encoder::reconfigure`)
- **CQP / CBR / VBR / LCVBR / QVBR / HQVBR / HQCBR** レート制御モード
- **ハンドラーベースのコールバック**設計: エンコード/デコード結果はワーカースレッドから `EncodeHandler` / `DecodeHandler` に通知される
- ビルド時に GitHub から AMF ヘッダーを自動取得 (`bindgen` 経由)

## バージョン情報

- crate 名: `shiguredo_amf`
- バージョン: 2026.3.0
- Rust Edition: 2024
- 最小 Rust バージョン: 1.88
- AMF SDK バージョン: v1.5.2
- ライセンス: Apache-2.0

## 動作要件

- Linux (x86_64)
- AMD GPU (RDNA 以降推奨)
- AMD GPU ドライバー (AMF ランタイムライブラリを含む)
- Vulkan ドライバー
- ビルド時: git

## コア API

### 公開構造体・トレイト・関数

| 型/関数 | 説明 | 主要 API |
|--------|------|---------|
| `AmfLibrary` | AMF ランタイムライブラリのプロセスシングルトン | `instance()`, `query_version() -> Result<(u16, u16, u16, u16), Error>` |
| `Encoder<H: EncodeHandler>` | AMF ハードウェアエンコーダー | `new(EncoderConfig, H)`, `alloc_surface()`, `encode(Surface, &EncodeOptions, H::UserData)`, `reconfigure(ReconfigureParams)`, `finish()` |
| `Decoder<H: DecodeHandler>` | AMF ハードウェアデコーダー | `new(DecoderConfig, H)`, `alloc_buffer(size)`, `decode(Buffer, H::UserData)`, `finish()` |
| `supported_codecs() -> Vec<CodecInfo>` | このバックエンドで利用可能なコーデック情報の一覧を返す (ランタイムがロードできない場合は全コーデック非対応を返す) | - |
| `BUILD_VERSION` | ビルド時の AMF バージョン文字列 (`&'static str`) | - |

`Encoder` / `Decoder` は内部でワーカースレッドを起動し、`encode()` / `decode()` で投入したフレームの結果は `EncodeHandler::on_encoded` / `DecodeHandler::on_decoded` に非同期で通知される。`finish()` は残りのフレームをハンドラーが受け取り終わるまで待機する。

### エンコーダー設定型

| 型 | 説明 | 主要メソッド |
|----|------|-------------|
| `EncoderConfig` | エンコーダー設定 | `new(CodecConfig, width, height, FrameFormat, framerate_num, framerate_den, RateControlMode)`, フィールド: `target_kbps`, `max_kbps`, `qpi`, `qpp`, `qpb`, `gop_pic_size` (全て `Option`) |
| `CodecConfig` | コーデック設定 enum | `H264(H264EncoderConfig)`, `Hevc(HevcEncoderConfig)`, `Av1(Av1EncoderConfig)` |
| `H264EncoderConfig` | H.264 エンコーダー固有設定 | `profile: Option<H264Profile>` |
| `HevcEncoderConfig` | HEVC エンコーダー固有設定 | `profile: Option<HevcProfile>` |
| `Av1EncoderConfig` | AV1 エンコーダー固有設定 | `profile: Option<Av1Profile>` |
| `H264Profile` | H.264 プロファイル | `Baseline`, `Main`, `High`, `ConstrainedBaseline`, `ConstrainedHigh` |
| `HevcProfile` | HEVC プロファイル | `Main`, `Main10` |
| `Av1Profile` | AV1 プロファイル | `Main` |
| `FrameFormat` | エンコーダー入力フレームフォーマット | `Nv12`, `Yv12`, `I420`, `Bgra`, `Argb`, `Rgba`, `Yuy2`, `Uyvy`, `P010`, `P012`, `P016`, `Y210`, `Ayuv`, `Y410`, `Y416`, `frame_size(width, height) -> Option<usize>` |
| `RateControlMode` | レート制御モード | `Cqp`, `Cbr`, `Vbr`, `LatencyConstrainedVbr`, `QualityVbr`, `HighQualityVbr`, `HighQualityCbr` |
| `EncodeOptions` | フレーム単位のエンコードオプション | `frame_type: u16` (デフォルト 0 = 自動。`frame_type` モジュール参照) |
| `ReconfigureParams` | 動的プロパティ再設定パラメータ | `framerate_num`, `framerate_den`, `target_kbps`, `max_kbps`, `qpi`, `qpp`, `qpb`, `gop_pic_size` (全て `Option`、`None` の項目は変更しない) |
| `EncodedFrame<T>` | エンコード済みフレーム | `buffer() -> &Buffer`, `picture_type() -> PictureType`, `user_data() -> &T`, `into_parts() -> (Buffer, T)` |
| `PictureType` | エンコード済みフレームのピクチャタイプ | `Idr`, `I`, `P`, `B`, `Unknown` |

### デコーダー設定型

| 型 | 説明 | 主要メソッド |
|----|------|-------------|
| `DecoderConfig` | デコーダー設定 | `codec: DecoderCodec` |
| `DecoderCodec` | デコーダーのコーデック種別 | `H264`, `Hevc`, `Av1` |
| `DecodedFrame<T>` | デコード済みフレーム | `surface() -> &Surface`, `user_data() -> &T`, `into_parts() -> (Surface, T)` |

### コーデック情報 (`codec_info`)

| 型/関数 | 説明 |
|--------|------|
| `supported_codecs() -> Vec<CodecInfo>` | このバックエンドで利用可能なコーデック情報の一覧 |
| `VideoCodecType` | コーデック種別 (`H264`, `Hevc`, `Av1`) |
| `CodecInfo` | コーデックごとの情報 (`codec: VideoCodecType`, `decoding: DecodingInfo`, `encoding: EncodingInfo`) |
| `DecodingInfo` | `supported: bool`, `hardware_accelerated: bool` |
| `EncodingInfo` | `supported`, `hardware_accelerated`, `supports_frame_reordering`, `supports_multi_pass`, `profiles: EncodingProfiles` |
| `EncodingProfiles` | `H264(Vec<H264EncodingProfile>)`, `Hevc(Vec<HevcEncodingProfile>)`, `Av1(Vec<Av1EncodingProfile>)`, `None` |
| `H264EncodingProfile` | `Baseline`, `ConstrainedBaseline`, `Main`, `High`, `ConstrainedHigh` |
| `HevcEncodingProfile` | `Main`, `Main10` |
| `Av1EncodingProfile` | `Main` |

## ハンドラートレイト

エンコード/デコード結果はワーカースレッドから通知される。利用者はトレイト実装またはクロージャラッパーを使う。

| トレイト/型 | 説明 |
|------------|------|
| `EncodeHandler` | `type UserData: Send + 'static`, `type Error: From<crate::Error> + Send + 'static`, `fn on_encoded(&mut self, Result<EncodedFrame<Self::UserData>, Self::Error>)` |
| `DecodeHandler` | `type UserData: Send + 'static`, `type Error: From<crate::Error> + Send + 'static`, `fn on_decoded(&mut self, Result<DecodedFrame<Self::UserData>, Self::Error>)` |
| `FnEncodeHandler<T, E = crate::Error>` | `FnMut(Result<EncodedFrame<T>, E>) + Send + 'static` を `EncodeHandler` にラップする |
| `FnDecodeHandler<T, E = crate::Error>` | `FnMut(Result<DecodedFrame<T>, E>) + Send + 'static` を `DecodeHandler` にラップする |

ハンドラーが `Send + 'static` を要求するのは、ワーカースレッドへ所有権を移して呼び出すため。`UserData` は `encode()` / `decode()` に渡したフレームと 1:1 で対応する。

### `frame_type` モジュール (vpl-rs 互換)

`EncodeOptions::frame_type` に指定するビットフラグ。AMF SDK 内部の picture type enum とは独立した値で、`Encoder::encode` 内で AMF の `force_picture_type` 等のプロパティに変換される。

| 定数 | 値 | 説明 |
|------|----|------|
| `UNKNOWN` | `0` | 自動 (フラグなし) |
| `I` | `0x0001` | I フレーム |
| `P` | `0x0002` | P フレーム |
| `B` | `0x0004` | B フレーム |
| `IDR` | `0x0020` | IDR フレーム (キーフレーム) |
| `REF` | `0x0040` | 参照フレーム |

現状の `Encoder::encode` は `frame_type & IDR != 0` のときに IDR を強制する。`I` / `P` / `B` / `REF` は vpl-rs と共通インターフェースを保つための定義で、現バージョンでは未使用。

## AMF オブジェクトラッパー (`amf` モジュール)

`*mut AMFSurface` などの生ポインタを `Acquire` / `Release` で RAII 管理するラッパー。vtable の関数ポインタは初期化時にすべて取り出して構造体メンバーに保持するため、メソッド呼び出しのたびに vtable 検証は行われない。

| 型 | 説明 | 主要メソッド |
|----|------|-------------|
| `Context` | `AMFContext` のラッパー | `terminate()`, `alloc_surface(memory_type, format, width, height)`, `alloc_buffer(memory_type, size)`, `unsafe init_vulkan(device: *mut c_void)`, `as_ptr()`, `into_raw()` |
| `Component` | `AMFComponent` のラッパー | `init(format, width, height)`, `re_init(width, height)`, `terminate()`, `drain()`, `flush()`, `unsafe submit_input(*mut AMFData)`, `unsafe query_output(*mut *mut AMFData)`, `get_context()`, `property_storage() -> Result<PropertyStorage, Error>` |
| `Surface` | `AMFSurface` のラッパー | `set_pts(amf_pts)`, `set_duration(amf_pts)`, `get_plane(AMF_PLANE_TYPE) -> Result<Plane, Error>`, `get_plane_at(index)`, `convert(AMF_MEMORY_TYPE)`, `property_storage()`, `as_ptr()`, `into_raw()` |
| `Buffer` | `AMFBuffer` のラッパー | `get_native() -> *mut c_void`, `get_size() -> amf_size`, `get_pts() -> amf_pts`, `get_duration() -> amf_pts`, `unsafe get_property(name_w, *mut AMFVariantStruct)`, `as_ptr()`, `into_raw()` |
| `Plane` | `AMFPlane` のラッパー | `get_native() -> *mut c_void`, `get_hpitch() -> amf_int32`, `get_vpitch() -> amf_int32`, `get_width() -> amf_int32`, `get_height() -> amf_int32`, `as_ptr()`, `into_raw()` |
| `PropertyStorage` | `AMFPropertyStorage` のラッパー | `set_property_int64(name, value)`, `set_property_size(name, width, height)`, `set_property_rate(name, num, den)`, `unsafe set_property(name_w, AMFVariantStruct)`, `as_ptr()`, `into_raw()` |

全ラッパーは `Clone` 実装で `Acquire` を呼び、`Drop` 実装で `Release` を呼ぶ。`unsafe from_raw(ptr)` (Acquire なし) と `unsafe from_raw_acquired(ptr)` (Acquire 付き) の 2 系統のコンストラクタを持つ。`into_raw()` は所有権を放棄して `Release` を呼ばずに生ポインタを返す。

すべて `unsafe impl Send` 済み (AMF オブジェクトはスレッドセーフであると AMF ドキュメントに記載されているため)。

## エラー型

| 型/関数 | 説明 |
|--------|------|
| `Error` | AMF 操作のエラー型 (`Debug + Display + std::error::Error`) |
| `Error::new_custom(function: &'static str, message: &str)` | カスタムエラーを作成 (`status` なし) |
| `Error::from_amf(AMF_RESULT, function)` | `AMF_RESULT` からエラーを作成 (内部の対応表を参照してメッセージを構築) |
| `Error::check(AMF_RESULT, function) -> Result<(), Error>` | `AMF_OK` 以外を `Err` にして返す |
| `Error::status() -> Option<AMF_RESULT>` | エラーに紐づく `AMF_RESULT` を返す (カスタムエラーは `None`) |

`Display` フォーマットは `function() failed[status=AMF_FAIL]: General failure (AMF_FAIL)` のように `function` / `status` / メッセージを含む。`Error::status()` で `AMF_RESULT` を取り出して呼び出し側で分岐できる。

主な `AMF_RESULT` バリアント (`ffi::AMF_RESULT` で公開): `AMF_OK`, `AMF_FAIL`, `AMF_INVALID_ARG`, `AMF_OUT_OF_MEMORY`, `AMF_NOT_SUPPORTED`, `AMF_WRONG_STATE`, `AMF_NO_DEVICE`, `AMF_EOF`, `AMF_REPEAT`, `AMF_INPUT_FULL`, `AMF_NEED_MORE_INPUT`, `AMF_DECODER_NO_FREE_SURFACES`, `AMF_VULKAN_FAILED` 等。

## FFI モジュール

`ffi` モジュールは内部 `sys` モジュールの再公開で、`#[doc(hidden)]` 属性が付いている。semver 保証の対象外。`AMFSurface`, `AMFBuffer`, `AMF_SURFACE_FORMAT`, `AMF_PLANE_TYPE`, `AMF_MEMORY_TYPE` 等の `bindgen` 生成型と AMF プロパティ文字列定数 (`sys::str::*`) を含む。

## コード例

### エンコード

```rust
use std::sync::{Arc, Mutex};

use shiguredo_amf::{
    CodecConfig, EncodeOptions, EncodedFrame, Encoder, EncoderConfig, FrameFormat,
    H264EncoderConfig, H264Profile, RateControlMode, ReconfigureParams, frame_type,
};

let mut config = EncoderConfig::new(
    CodecConfig::H264(H264EncoderConfig {
        profile: Some(H264Profile::High),
    }),
    1920, // width
    1080, // height
    FrameFormat::Nv12,
    30, // framerate_num
    1,  // framerate_den
    RateControlMode::Cbr,
);
config.target_kbps = Some(5_000);

// クロージャをハンドラーにラップする
let encoded = Arc::new(Mutex::new(Vec::new()));
let e = encoded.clone();
let mut encoder = Encoder::new(config, move |frame: EncodedFrame<()>| {
    e.lock().unwrap().push(frame);
})?;

// Surface を確保してフレームデータをコピーし、エンコードする
let surface = encoder.alloc_surface()?;
// ここで surface の Y/UV プレーンに NV12 フレームを書き込む...
let options = EncodeOptions { frame_type: frame_type::UNKNOWN };
encoder.encode(surface, &options, ())?;

// 残りのフレームをすべて取得する (Drain → ワーカー完了待ち)
encoder.finish()?;

for frame in encoded.lock().unwrap().iter() {
    println!("encoded bytes: {}", frame.buffer().get_size());
    println!("pts: {}", frame.buffer().get_pts());
    println!("picture type: {:?}", frame.picture_type());
}
```

`Encoder::new` に渡すクロージャは `FnMut(Result<EncodedFrame<T>, E>) + Send + 'static` であれば暗黙的に `FnEncodeHandler` 相当として扱える。`UserData` 型 (上記の `()` の位置) はフレーム単位の任意ユーザーデータで、`encode()` で渡した値が `EncodedFrame::user_data()` から取り出せる。

### デコード

```rust
use std::sync::{Arc, Mutex};

use shiguredo_amf::{DecodedFrame, Decoder, DecoderCodec, DecoderConfig};

let config = DecoderConfig { codec: DecoderCodec::H264 };
let decoded = Arc::new(Mutex::new(Vec::new()));
let d = decoded.clone();
let mut decoder = Decoder::new(config, move |frame: DecodedFrame<()>| {
    d.lock().unwrap().push(frame);
})?;

// Buffer を確保してビットストリームデータをコピーし、デコードする
let buffer = decoder.alloc_buffer(bitstream_data.len())?;
// ここで buffer.get_native() に bitstream_data を書き込む...
decoder.decode(buffer, ())?;

// 残りのフレームをすべて取得する (Drain → ワーカー完了待ち)
decoder.finish()?;
drop(decoder);

for frame in decoded.lock().unwrap().iter() {
    let y_plane = frame.surface()
        .get_plane(shiguredo_amf::ffi::AMF_PLANE_TYPE::AMF_PLANE_Y)?;
    println!("decoded: {}x{}", y_plane.get_width(), y_plane.get_height());
}
```

デコーダーは初期化時に解像度 `0x0` で AMF コンポーネントを Init するため、ビットストリームのシーケンスヘッダーから解像度が自動検出される。出力 `Surface` は内部で `convert(AMF_MEMORY_HOST)` 済みのため、`get_plane()` 経由でホストメモリから直接読み出せる。

### エンコード中の動的プロパティ再設定

```rust
use shiguredo_amf::ReconfigureParams;

// フレームレートとビットレートを変更する
encoder.reconfigure(ReconfigureParams {
    framerate_num: Some(15),
    framerate_den: Some(1),
    target_kbps: Some(3_000),
    ..ReconfigureParams::default()
})?;
```

`reconfigure` で変更できる項目は codec ごとに異なる:

- **H.264**: `framerate_num` / `framerate_den` / `target_kbps` / `max_kbps` / `qpi` / `qpp` / `qpb` / `gop_pic_size`
- **H.265**: `framerate_num` / `framerate_den` / `target_kbps` / `max_kbps` / `qpi` / `qpp` (B フレーム用 QP プロパティが存在しないため `qpb` / `gop_pic_size` は無視)
- **AV1**: `framerate_num` / `framerate_den` / `target_kbps` / `max_kbps` / `qpi` / `qpp` / `qpb` / `gop_pic_size`

`framerate_num` と `framerate_den` は必ず同時に指定する。片方だけ指定すると `Err` を返す。`None` の項目はそのまま維持される。

### IDR フレーム強制

```rust
use shiguredo_amf::{EncodeOptions, frame_type};

let surface = encoder.alloc_surface()?;
// ...
encoder.encode(surface, &EncodeOptions {
    frame_type: frame_type::IDR | frame_type::I | frame_type::REF,
}, ())?;
```

IDR 強制時は内部でコーデック別に AMF の `FORCE_PICTURE_TYPE` / `FORCE_FRAME_TYPE` プロパティを設定し、H.264 では `INSERT_SPS` / `INSERT_PPS`、HEVC では `INSERT_HEADER`、AV1 では `FORCE_INSERT_SEQUENCE_HEADER` を同時に有効化する (キーフレームに SPS/PPS 相当のヘッダーが付与される)。

### コーデック対応状況の確認

```rust
use shiguredo_amf::supported_codecs;

let codecs = supported_codecs();
for info in &codecs {
    println!("{:?}:", info.codec);
    println!(
        "  decoding: supported={}, hw_accel={}",
        info.decoding.supported, info.decoding.hardware_accelerated
    );
    println!(
        "  encoding: supported={}, hw_accel={}",
        info.encoding.supported, info.encoding.hardware_accelerated
    );
}
```

`supported_codecs()` は AMF ランタイムをロードして実際に `CreateComponent` を試行する。AMF ランタイムがロードできない環境では全コーデックが非対応として返される。

### AMF ランタイムバージョンの取得

```rust
use shiguredo_amf::AmfLibrary;

let lib = AmfLibrary::instance();
let (major, minor, release, build) = lib.query_version()?;
println!("AMF runtime: {major}.{minor}.{release}.{build}");
```

`AmfLibrary::instance()` は `LazyLock` によるプロセスシングルトン。初回呼び出し時に `libamfrt64.so.1` を `dlopen` してファクトリを取得する。

### カスタムハンドラートレイトの実装

```rust
use shiguredo_amf::{EncodeHandler, EncodedFrame};

struct MyHandler {
    /* 状態 */
}

#[derive(Debug)]
struct MyError(shiguredo_amf::Error);

impl From<shiguredo_amf::Error> for MyError {
    fn from(e: shiguredo_amf::Error) -> Self { MyError(e) }
}

impl EncodeHandler for MyHandler {
    type UserData = u64; // 例: フレーム ID
    type Error = MyError;

    fn on_encoded(&mut self, result: Result<EncodedFrame<u64>, MyError>) {
        match result {
            Ok(frame) => {
                let frame_id = *frame.user_data();
                /* frame.buffer() を処理する */
            }
            Err(e) => eprintln!("encode error: {:?}", e),
        }
    }
}
```

`UserData` は `encode()` 時に渡した値が `EncodedFrame::user_data()` から取り出せる。`Error` 型を絞り込みたい場合のためのインターフェース。クロージャで十分なら `FnEncodeHandler::new(...)` で済む。

## エンコーダー入力フォーマット (`FrameFormat`)

| バリアント | 説明 |
|---|---|
| `Nv12` | Semi-Planar YUV 4:2:0 8bit |
| `Yv12` | Planar YUV 4:2:0 8bit (Y+V+U) |
| `I420` | Planar YUV 4:2:0 8bit (Y+U+V) |
| `Bgra` | Packed BGRA 8bit |
| `Argb` | Packed ARGB 8bit |
| `Rgba` | Packed RGBA 8bit |
| `Yuy2` | Packed YUV 4:2:2 8bit (YUY2) |
| `Uyvy` | Packed YUV 4:2:2 8bit (UYVY) |
| `P010` | Semi-Planar YUV 4:2:0 10bit |
| `P012` | Semi-Planar YUV 4:2:0 12bit (16bit 格納) |
| `P016` | Semi-Planar YUV 4:2:0 16bit |
| `Y210` | Packed YUV 4:2:2 10bit (16bit 格納) |
| `Ayuv` | Packed YUV 4:4:4 8bit |
| `Y410` | Packed YUV 4:4:4 10bit |
| `Y416` | Packed YUV 4:4:4 16bit |

`FrameFormat::frame_size(width, height) -> Option<usize>` で各フォーマットのフレームバイト数を計算できる (オーバーフロー時は `None`)。

## レート制御モード (`RateControlMode`)

| モード | 説明 |
|---|---|
| `Cqp` | 固定 QP |
| `Cbr` | 固定ビットレート |
| `Vbr` | ピーク制約付き可変ビットレート |
| `LatencyConstrainedVbr` | レイテンシ制約付き可変ビットレート |
| `QualityVbr` | 品質 VBR |
| `HighQualityVbr` | 高品質 VBR |
| `HighQualityCbr` | 高品質 CBR |

H.264 / HEVC / AV1 で AMF SDK 内部の enum 値順序が異なるが、`shiguredo_amf` 側ではこの enum で抽象化されている。

## EncoderConfig のバリデーション

`Encoder::new` で以下を検証する:

| 検査 | エラーメッセージの内容 |
|------|----------------------|
| `width == 0 \|\| height == 0` | `invalid resolution: WxH (must be non-zero)` |
| `width > i32::MAX \|\| height > i32::MAX` | `invalid resolution: WxH (exceeds i32::MAX)` |
| `width` または `height` が奇数 | `invalid resolution: WxH (must be even)` |
| `framerate_num == 0 \|\| framerate_den == 0` | `invalid framerate: N/D (must be non-zero)` |

## スレッド設計

- `AmfLibrary` はプロセスシングルトン (`LazyLock<AmfLibrary>`)。内部の `Mutex` で `AMFFactory` を遅延初期化する。
- `Encoder<H>` / `Decoder<H>` は内部で `amf-encoder-worker` / `amf-decoder-worker` という名前のワーカースレッドを起動し、`mpsc::channel` で `Submit(UserData)` / `Finish` コマンドを受け取る。
- `encode()` / `decode()` はメインスレッドから `SubmitInput` を呼び (必要に応じて `AMF_INPUT_FULL` / `AMF_REPEAT` / `AMF_DECODER_NO_FREE_SURFACES` で 1ms スリープしながら最大 100 回までリトライ)、その後ワーカーへ `UserData` を送る。
- ワーカーは `pending` キューにマッチするように `QueryOutput` の結果を保持し、対応が取れたところで `on_encoded` / `on_decoded` を呼ぶ。
- `finish()` は `Drain` を呼んだあと `Finish(sync_channel)` をワーカーに送り、最大 5 秒で完了を待つ。
- `Drop` 実装は `cmd_tx` を落としてワーカーを停止させ、`Component::terminate()` / `Context::terminate()` を呼んでから AMF オブジェクトを解放する。

## 公開モジュール

| モジュール | 説明 |
|-----------|------|
| `amf` | AMF オブジェクトの RAII ラッパー (`Context`, `Component`, `Surface`, `Buffer`, `Plane`, `PropertyStorage`) |
| `ffi` | `#[doc(hidden)]`。`sys` モジュールの再公開。semver 保証対象外 |

## docs.rs 向けビルド

AMF ランタイムがロードできない環境 (CI、docs.rs 等) では `DOCS_RS=1 cargo doc --no-deps` でドキュメント生成のみ可能。
