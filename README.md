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
- AMF によるハードウェアデコード (H.264 / H.265 / AV1)
- AMF ランタイムライブラリの実行時動的ロード (ビルド時のリンク不要)
- Vulkan バックエンドによる GPU 処理
- エンコード入力フォーマット選択 (NV12 / YV12 / I420 / BGRA / ARGB / RGBA / YUY2 / UYVY / P010 / P012 / P016 / Y210 / AYUV / Y410 / Y416)
- デコード出力は NV12 フォーマット
- フレーム単位のエンコードオプション (IDR フレーム強制)
- エンコード中の動的プロパティ再設定 (`reconfigure`)
- CQP / CBR / VBR / LCVBR / QVBR / HQVBR / HQCBR レート制御モード
- ビルド時に GitHub から AMF ヘッダーを自動取得

## 動作要件

- Linux (x86_64)
- AMD GPU (RDNA 以降推奨)
- AMD GPU ドライバー (AMF ランタイムライブラリを含む)
- Vulkan ドライバー
- ビルド時: git

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
use std::sync::{Arc, Mutex};

use shiguredo_amf::{
    CodecConfig, EncodeOptions, Encoder, EncoderConfig, FrameFormat,
    H264EncoderConfig, H264Profile, RateControlMode, ReconfigureParams, frame_type,
};

let mut config = EncoderConfig::new(
    CodecConfig::H264(H264EncoderConfig {
        profile: Some(H264Profile::High),
    }),
    1920,                    // width
    1080,                    // height
    FrameFormat::Nv12,
    30,                      // framerate_num
    1,                       // framerate_den
    RateControlMode::Cbr,
);
config.target_kbps = Some(5_000);

let encoded = Arc::new(Mutex::new(Vec::new()));
let e = encoded.clone();
let mut encoder = Encoder::new(config, move |frame, _: ()| {
    e.lock().unwrap().push(frame);
})?;

// エンコード中に動的プロパティを再設定
encoder.reconfigure(ReconfigureParams {
    framerate_num: Some(15),
    framerate_den: Some(1),
    target_kbps: Some(3_000),
    ..ReconfigureParams::default()
})?;

// フレームデータをエンコード
let options = EncodeOptions { frame_type: frame_type::UNKNOWN };
encoder.encode(&frame_data, &options, ())?;

// IDR フレームを強制してエンコード
encoder.encode(&frame_data, &EncodeOptions {
    frame_type: frame_type::IDR | frame_type::I | frame_type::REF,
}, ())?;

// 残りのフレームをすべて取得する
encoder.finish()?;

// エンコード済みフレームを確認
for encoded in encoded.lock().unwrap().iter() {
    println!("encoded bytes: {}", encoded.data().len());
    println!("pts: {}", encoded.pts());
    println!("picture type: {:?}", encoded.picture_type());
}
```

`reconfigure` で変更できる項目は codec ごとに異なります。

- H.264: `framerate_num` / `framerate_den` / `target_kbps` / `max_kbps` / `qpi` / `qpp` / `qpb` / `gop_pic_size`
- H.265: `framerate_num` / `framerate_den` / `target_kbps` / `max_kbps` / `qpi` / `qpp`
- AV1: `framerate_num` / `framerate_den` / `target_kbps` / `max_kbps` / `qpi` / `qpp` / `qpb` / `gop_pic_size`

`framerate_num` と `framerate_den` は必ず同時に指定してください。

### デコード

```rust
use std::sync::{Arc, Mutex};

use shiguredo_amf::{Decoder, DecoderCodec, DecoderConfig};

let config = DecoderConfig {
    codec: DecoderCodec::H264,
};
let decoded = Arc::new(Mutex::new(Vec::new()));
let d = decoded.clone();
let mut decoder = Decoder::new(config, move |frame, _: ()| {
    d.lock().unwrap().push(frame);
})?;

// ビットストリームデータをデコード
decoder.decode(&bitstream_data, ())?;

// 残りのフレームをすべて取得する
decoder.finish()?;
drop(decoder);

// デコード済みフレームを確認 (NV12 フォーマット)
for frame in decoded.lock().unwrap().iter() {
    println!("decoded: {}x{}, {} bytes", frame.width(), frame.height(), frame.data().len());
}
```

### コーデック対応状況の確認

```rust
use shiguredo_amf::supported_codecs;

let codecs = supported_codecs();
for info in &codecs {
    println!("{:?}:", info.codec);
    println!("  decoding: supported={}, hw_accel={}",
        info.decoding.supported, info.decoding.hardware_accelerated);
    println!("  encoding: supported={}, hw_accel={}",
        info.encoding.supported, info.encoding.hardware_accelerated);
}
```

AMF ランタイムがロードできない環境では、全コーデックが非対応として返されます。

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

## サポートフォーマット

### エンコード入力フォーマット (`FrameFormat`)

| フォーマット | `FrameFormat` | 説明 |
|---|---|---|
| NV12 | `FrameFormat::Nv12` | Semi-Planar YUV 4:2:0 8bit |
| YV12 | `FrameFormat::Yv12` | Planar YUV 4:2:0 8bit (Y+V+U) |
| I420 | `FrameFormat::I420` | Planar YUV 4:2:0 8bit (Y+U+V) |
| BGRA | `FrameFormat::Bgra` | Packed BGRA 8bit |
| ARGB | `FrameFormat::Argb` | Packed ARGB 8bit |
| RGBA | `FrameFormat::Rgba` | Packed RGBA 8bit |
| YUY2 | `FrameFormat::Yuy2` | Packed YUV 4:2:2 8bit |
| UYVY | `FrameFormat::Uyvy` | Packed YUV 4:2:2 8bit |
| P010 | `FrameFormat::P010` | Semi-Planar YUV 4:2:0 10bit |
| P012 | `FrameFormat::P012` | Semi-Planar YUV 4:2:0 12bit (16bit 格納) |
| P016 | `FrameFormat::P016` | Semi-Planar YUV 4:2:0 16bit |
| Y210 | `FrameFormat::Y210` | Packed YUV 4:2:2 10bit (16bit 格納) |
| AYUV | `FrameFormat::Ayuv` | Packed YUV 4:4:4 8bit |
| Y410 | `FrameFormat::Y410` | Packed YUV 4:4:4 10bit |
| Y416 | `FrameFormat::Y416` | Packed YUV 4:4:4 16bit |

### デコード出力フォーマット

| フォーマット | 説明 |
|---|---|
| NV12 | Semi-Planar YUV 4:2:0 8bit |

## サポートプロファイル

### H.264 (`H264Profile`)

| プロファイル | `H264Profile` |
|------------|--------------|
| Baseline   | `H264Profile::Baseline` |
| Main       | `H264Profile::Main` |
| High       | `H264Profile::High` |
| Constrained Baseline | `H264Profile::ConstrainedBaseline` |
| Constrained High | `H264Profile::ConstrainedHigh` |

### H.265 (`HevcProfile`)

| プロファイル | `HevcProfile` |
|------------|--------------|
| Main       | `HevcProfile::Main` |
| Main 10    | `HevcProfile::Main10` |

### AV1 (`Av1Profile`)

| プロファイル | `Av1Profile` |
|------------|-------------|
| Main       | `Av1Profile::Main` |

## レート制御モード (`RateControlMode`)

| モード | `RateControlMode` | 説明 |
|---|---|---|
| CQP  | `RateControlMode::Cqp` | 固定 QP |
| CBR  | `RateControlMode::Cbr` | 固定ビットレート |
| VBR  | `RateControlMode::Vbr` | ピーク制約付き可変ビットレート |
| LCVBR | `RateControlMode::LatencyConstrainedVbr` | レイテンシ制約付き可変ビットレート |
| QVBR | `RateControlMode::QualityVbr` | 品質 VBR |
| HQVBR | `RateControlMode::HighQualityVbr` | 高品質 VBR |
| HQCBR | `RateControlMode::HighQualityCbr` | 高品質 CBR |

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
