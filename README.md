# amf-rs

[![crates.io](https://img.shields.io/crates/v/shiguredo_amf.svg)](https://crates.io/crates/shiguredo_amf)
[![docs.rs](https://docs.rs/shiguredo_amf/badge.svg)](https://docs.rs/shiguredo_amf)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![GitHub Actions](https://github.com/shiguredo/amf-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/shiguredo/amf-rs/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white)](https://discord.gg/shiguredo)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## 概要

[AMD AMF (Advanced Media Framework)](https://github.com/GPUOpen-LibrariesAndSDKs/AMF) を利用したハードウェアビデオエンコーダーおよびデコーダーの Rust バインディングです。

AMF ランタイムライブラリ (libamfrt64.so.1) は AMD GPU ドライバーに同梱されており、dlopen で動的にロードするため、ビルド時のリンクは不要です。

## 特徴

- AMF によるハードウェアエンコード (H.264 / H.265 / AV1)
- AMF によるハードウェアデコード (H.264 / H.265 / AV1 / VP9)
- AMF ランタイムライブラリの実行時動的ロード (ビルド時のリンク不要)
- Vulkan バックエンドによる GPU 処理
- 複数 GPU 環境での Vulkan デバイス選択
- エンコード入力フォーマット (NV12 / YV12 / I420 / BGRA / ARGB / RGBA / YUY2 / P010 など)
- デコード出力は NV12 フォーマット
- デコーダーの低遅延モード、リオーダーモード、サーフェスプールサイズの指定
- フレーム単位のエンコードオプション (IDR / I / P フレームの強制)
- B フレーム (H.264 / AV1、GPU と AMF が対応している場合)
- エンコード中の動的プロパティ再設定 (`reconfigure`)
- レート制御モード (CQP / CBR / VBR / LCVBR / QVBR / HQVBR / HQCBR)
- レート制御時の QP 範囲指定
- エンコーダーの用途 (`EncoderUsage`) と品質プリセット (`QualityPreset`) の指定
- エンコーダー / デコーダーの統計値取得
- ビルド時に GitHub から AMF ヘッダーを自動取得

## 動作要件

- Linux (x86_64)
- AMD GPU (RDNA 以降推奨)
- AMD GPU ドライバー (AMF ランタイムライブラリを含む)
- Vulkan ドライバー
- ビルド時: git、libclang (bindgen 依存、Ubuntu では `libclang-dev` パッケージ)

対応プラットフォームは Linux x86_64 のみです。Windows と macOS には対応していません。

### 動作確認済み環境

- AMD Ryzen 9 7940HS
- AMD Radeon 780M Graphics (Phoenix1)
- Ubuntu 26.04.1 LTS
- AMF ランタイム 1.5.3.0 (AMF SDK 1.5.2)
- Vulkan: RADV PHOENIX (Mesa 26.0.8)

## ビルド

```bash
cargo build
```

ビルド時に GitHub から AMF ヘッダーを自動取得します。

### docs.rs 向けビルド

AMF ランタイムがない環境では、docs.rs 向けのドキュメント生成のみ可能です。

```bash
DOCS_RS=1 cargo doc --no-deps
```

## 使い方

### エンコード

```rust
use std::sync::mpsc;

use shiguredo_amf::{
    AmfLibrary, CodecConfig, EncodeOptions, EncodedFrame, Encoder, EncoderConfig,
    FnEncodeHandler, FrameFormat, H264EncoderConfig, H264Profile, RateControlMode, frame_type,
};

let context = AmfLibrary::instance().create_context()?;

let mut config = EncoderConfig::new(
    CodecConfig::H264(H264EncoderConfig {
        profile: Some(H264Profile::High),
        ..Default::default()
    }),
    1920, // width
    1080, // height
    FrameFormat::Nv12,
    30, // framerate_num
    1,  // framerate_den
    RateControlMode::Cbr,
);
config.target_kbps = Some(5_000);

// エンコード結果はワーカースレッドからチャネルで受け取る
let (frame_tx, frame_rx) = mpsc::channel();
let mut encoder = Encoder::new(
    context,
    config,
    FnEncodeHandler::new(
        move |result: Result<EncodedFrame<()>, shiguredo_amf::Error>| {
            let _ = frame_tx.send(result);
        },
    ),
)?;

// Surface を確保してフレームデータを書き込み、エンコードする
let surface = encoder.alloc_surface()?;
// ここでフレームデータを surface の Y/UV プレーンに書き込む
// ...
// 最初のフレームなので IDR を強制する
encoder.encode(
    surface,
    EncodeOptions {
        frame_type: frame_type::IDR,
    },
    (),
)?;

// 残りのフレームをすべて取得する
encoder.finish()?;

for result in frame_rx.try_iter() {
    let frame = result?;
    println!("encoded bytes: {}", frame.buffer().get_size());
}
```

エンコード結果はワーカースレッドからハンドラーへ通知されます。`FnEncodeHandler` を使うと
`mpsc::channel` でメインスレッドへ送れます。

### デコード

```rust
use std::sync::mpsc;

use shiguredo_amf::{
    AmfLibrary, DecodedFrame, Decoder, DecoderCodec, DecoderConfig, FnDecodeHandler,
};

let context = AmfLibrary::instance().create_context()?;
let config = DecoderConfig::new(DecoderCodec::H264);

// デコード結果はワーカースレッドからチャネルで受け取る
let (frame_tx, frame_rx) = mpsc::channel();
let mut decoder = Decoder::new(
    context,
    config,
    FnDecodeHandler::new(
        move |result: Result<DecodedFrame<()>, shiguredo_amf::Error>| {
            let _ = frame_tx.send(result);
        },
    ),
)?;

// 1 フレーム分の H.264 ビットストリームを読み込む
let bitstream_data = std::fs::read("input.h264")?;

// Buffer を確保してビットストリームデータをコピーし、デコードする
let mut buffer = decoder.alloc_buffer(bitstream_data.len())?;
unsafe { buffer.as_mut_slice() }?[..bitstream_data.len()].copy_from_slice(&bitstream_data);
decoder.decode(buffer, ())?;

// 残りのフレームをすべて取得する
decoder.finish()?;

for result in frame_rx.try_iter() {
    let frame = result?;
    let y_plane = frame.surface().get_plane(shiguredo_amf::PlaneType::Y)?;
    println!("decoded: {}x{}", y_plane.get_width(), y_plane.get_height());
}
```

`decode()` には 1 フレーム分のビットストリームを格納した `Buffer` を渡します。
出力は NV12 のホストメモリです。遅延やサーフェスプールは `DecoderConfig` の
`low_latency` / `reorder_mode` / `surface_pool_size` で調整できます。

API の詳細は [docs.rs](https://docs.rs/shiguredo_amf) を参照してください。

### 動的再設定

`Encoder::reconfigure` で、エンコード中にフレームレートやビットレートを変更できます。

```rust
use shiguredo_amf::ReconfigureParams;

encoder.reconfigure(ReconfigureParams {
    framerate_num: Some(15),
    framerate_den: Some(1),
    target_kbps: Some(3_000),
    ..ReconfigureParams::default()
})?;
```

変更できる項目はコーデックによって異なります。詳細は
[docs.rs](https://docs.rs/shiguredo_amf) を参照してください。

### 統計値

`Encoder::stats()` / `Decoder::stats()` で、処理の進行状況を取得できます。

```rust
let stats = encoder.stats();
println!("accepted: {}", stats.total_encode_count.get());
println!("output: {}", stats.total_output_frame_count.get());
println!("errors: {}", stats.total_error_count.get());
println!("bits: {}", stats.total_output_bit_count.get());
println!("reconfigured: {}", stats.total_reconfigure_count.get());
println!("in flight: {}", stats.in_flight_frames.get());
```

デコーダーも `Decoder::stats()` で同じように取得できます。

### コーデック対応状況の確認

```rust
use shiguredo_amf::supported_codecs;

let codecs = supported_codecs().expect("failed to query codec support");
for info in &codecs {
    println!("{:?}:", info.codec);
    match &info.decoding {
        Some(decoding) => println!("  decoding: acceleration={:?}", decoding.acceleration),
        None => println!("  decoding: not supported"),
    }
    match &info.encoding {
        Some(encoding) => println!(
            "  encoding: acceleration={:?}, profile={:?}, level={:?}, frame_reordering={:?}",
            encoding.acceleration,
            encoding.max_profile,
            encoding.max_level,
            encoding.supports_frame_reordering
        ),
        None => println!("  encoding: not supported"),
    }
}
```

`supported_codecs()` は H.264 / H.265 / AV1 / VP9 の各コーデックについて、次の情報を返します。

- デコード / エンコードの可否
- アクセラレーション種別
- エンコードの最大プロファイルと最大レベル
- B フレーム (フレームリオーダリング) の対応可否
- 入出力の解像度範囲、対応フォーマット、対応メモリ種別、インターレース対応
- AMF が返すその他のプロパティ

CLI で確認する場合は [amf-check](#amf-check) を利用してください。

## サポートコーデック

### エンコード

| コーデック | `CodecConfig` |
|-----------|--------------|
| H.264     | `CodecConfig::H264(H264EncoderConfig)` |
| H.265     | `CodecConfig::Hevc(HevcEncoderConfig)` |
| AV1       | `CodecConfig::Av1(Av1EncoderConfig)` |

### デコード

| コーデック | `DecoderCodec` |
|-----------|---------------|
| H.264     | `DecoderCodec::H264` |
| H.265     | `DecoderCodec::Hevc` |
| AV1       | `DecoderCodec::Av1` |
| VP9       | `DecoderCodec::Vp9` |

VP9 はデコードのみに対応します。

## サポートフォーマット

エンコード入力フォーマットは `FrameFormat` で指定します。8bit の `Nv12` / `Yv12` /
`I420` / `Bgra` / `Argb` / `Rgba` / `Yuy2` は利用できます。10bit の `P010` は HEVC では
`HevcProfile::Main10` が必要で、AV1 は `Av1Profile::Main` で利用できます。
`Uyvy` / `Ayuv` / `Y210` / `Y410` の利用可否は GPU と AMF の実装に依存し、
`P012` / `P016` / `Y416` は利用できません。

デコード出力は `NV12` だけです。

## サポートプロファイル

H.264 は `Baseline` / `Main` / `High` / `ConstrainedBaseline` / `ConstrainedHigh`、
HEVC は `Main` / `Main10`、AV1 は `Main` に対応します。
それぞれ `H264Profile` / `HevcProfile` / `Av1Profile` で指定します。

## 主な設定

- レート制御: `EncoderConfig::rate_control_mode`
- QP 範囲: `H264EncoderConfig::qp_range` など
- 用途と品質: `EncoderConfig::usage` と `EncoderConfig::quality_preset`
- B フレーム: `EncoderConfig::b_pic_pattern`
- デコーダーの遅延: `DecoderConfig` の `low_latency` / `reorder_mode` / `surface_pool_size`

## サンプル

引数のライブラリには [noargs](https://github.com/sile/noargs) を利用しています。

### amf-check

AMF がこの環境で利用できるかを確認する CLI です。
AMF ランタイムのロード、バージョン照会、コーデック対応状況の表示、エンコーダー / デコーダーの初期化確認を行います。

```bash
cargo run -p amf-check
```

エンコーダー初期化時の解像度を指定する場合:

```bash
cargo run -p amf-check -- --width 1280 --height 720
```

## ライセンス

Apache License 2.0

```text
Copyright 2026-2026, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

## AMD AMF

<https://github.com/GPUOpen-LibrariesAndSDKs/AMF>

<https://gpuopen.com/advanced-media-framework/>
