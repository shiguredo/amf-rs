//! コーデック情報の照会

use crate::AmfLibrary;
use crate::amf::Context;
use crate::sys;

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

/// コーデック種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodecType {
    /// H.264 / AVC
    H264,
    /// H.265 / HEVC
    Hevc,
    /// AV1
    Av1,
}

impl VideoCodecType {
    /// すべてのコーデック種別を返す
    fn all() -> &'static [Self] {
        &[Self::H264, Self::Hevc, Self::Av1]
    }
}

/// コーデックごとの情報
#[derive(Debug, Clone, PartialEq)]
pub struct CodecInfo {
    /// コーデック種別
    pub codec: VideoCodecType,
    /// デコード情報
    pub decoding: DecodingInfo,
    /// エンコード情報
    pub encoding: EncodingInfo,
}

/// デコード情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodingInfo {
    /// デコードが可能か
    pub supported: bool,
    /// ハードウェアアクセラレーションが利用可能か
    pub hardware_accelerated: bool,
}

/// エンコード情報
#[derive(Debug, Clone, PartialEq)]
pub struct EncodingInfo {
    /// エンコードが可能か
    pub supported: bool,
    /// ハードウェアアクセラレーションが利用可能か
    pub hardware_accelerated: bool,
    /// フレームリオーダリング (B フレーム) をサポートするか
    pub supports_frame_reordering: bool,
    /// マルチパスエンコードをサポートするか
    pub supports_multi_pass: bool,
    /// コーデック固有のプロファイル情報
    pub profiles: EncodingProfiles,
}

/// コーデック固有のエンコードプロファイル情報
#[derive(Debug, Clone, PartialEq)]
pub enum EncodingProfiles {
    /// H.264 プロファイル一覧
    H264(Vec<H264EncodingProfile>),
    /// HEVC プロファイル一覧
    Hevc(Vec<HevcEncodingProfile>),
    /// AV1 プロファイル一覧
    Av1(Vec<Av1EncodingProfile>),
    /// プロファイル情報なし (エンコード非対応)
    None,
}

/// H.264 エンコードプロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264EncodingProfile {
    /// Baseline
    Baseline,
    /// Constrained Baseline
    ConstrainedBaseline,
    /// Main
    Main,
    /// High
    High,
    /// Constrained High
    ConstrainedHigh,
}

/// HEVC エンコードプロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcEncodingProfile {
    /// Main
    Main,
    /// Main10
    Main10,
}

/// AV1 エンコードプロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Av1EncodingProfile {
    /// Main
    Main,
}

// ---------------------------------------------------------------------------
// 公開関数
// ---------------------------------------------------------------------------

/// このバックエンドで利用可能なコーデック情報の一覧を返す
///
/// AMF ランタイムを初期化できない場合は全コーデック非対応を返す。
pub fn supported_codecs() -> Vec<CodecInfo> {
    let codecs = VideoCodecType::all();

    let context = match AmfLibrary::instance().create_context() {
        Ok(context) => context,
        Err(_) => {
            return codecs.iter().map(|&codec| not_supported(codec)).collect();
        }
    };

    codecs
        .iter()
        .map(|&codec| {
            let enc_supported = try_create_encoder(&context, codec);
            let dec_supported = try_create_decoder(&context, codec);

            CodecInfo {
                codec,
                decoding: DecodingInfo {
                    supported: dec_supported,
                    hardware_accelerated: dec_supported,
                },
                encoding: if enc_supported {
                    encoding_info(codec)
                } else {
                    EncodingInfo {
                        supported: false,
                        hardware_accelerated: false,
                        supports_frame_reordering: false,
                        supports_multi_pass: false,
                        profiles: EncodingProfiles::None,
                    }
                },
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 内部実装
// ---------------------------------------------------------------------------

/// 全フィールドが非対応の CodecInfo を返す
fn not_supported(codec: VideoCodecType) -> CodecInfo {
    CodecInfo {
        codec,
        decoding: DecodingInfo {
            supported: false,
            hardware_accelerated: false,
        },
        encoding: EncodingInfo {
            supported: false,
            hardware_accelerated: false,
            supports_frame_reordering: false,
            supports_multi_pass: false,
            profiles: EncodingProfiles::None,
        },
    }
}

/// エンコード対応時のプロファイル情報をハードコードで返す
///
/// AMF SDK にはプロファイル一覧を問い合わせる API がないため、
/// AMF SDK ヘッダーに定義されている値をもとに静的に返す。
fn encoding_info(codec: VideoCodecType) -> EncodingInfo {
    match codec {
        VideoCodecType::H264 => EncodingInfo {
            supported: true,
            hardware_accelerated: true,
            supports_frame_reordering: true,
            supports_multi_pass: false,
            profiles: EncodingProfiles::H264(vec![
                H264EncodingProfile::Baseline,
                H264EncodingProfile::ConstrainedBaseline,
                H264EncodingProfile::Main,
                H264EncodingProfile::High,
                H264EncodingProfile::ConstrainedHigh,
            ]),
        },
        VideoCodecType::Hevc => EncodingInfo {
            supported: true,
            hardware_accelerated: true,
            supports_frame_reordering: true,
            supports_multi_pass: false,
            profiles: EncodingProfiles::Hevc(vec![
                HevcEncodingProfile::Main,
                HevcEncodingProfile::Main10,
            ]),
        },
        VideoCodecType::Av1 => EncodingInfo {
            supported: true,
            hardware_accelerated: true,
            supports_frame_reordering: false,
            supports_multi_pass: false,
            profiles: EncodingProfiles::Av1(vec![Av1EncodingProfile::Main]),
        },
    }
}

/// 指定コーデックのエンコーダーを作成できるかを返す
fn try_create_encoder(context: &Context, codec: VideoCodecType) -> bool {
    let component_id = match codec {
        VideoCodecType::H264 => sys::str::AMFVideoEncoderVCE_AVC,
        VideoCodecType::Hevc => sys::str::AMFVideoEncoder_HEVC,
        VideoCodecType::Av1 => sys::str::AMFVideoEncoder_AV1,
    };
    try_create_component(context, component_id)
}

/// 指定コーデックのデコーダーを作成できるかを返す
fn try_create_decoder(context: &Context, codec: VideoCodecType) -> bool {
    let component_id = match codec {
        VideoCodecType::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
        VideoCodecType::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
        VideoCodecType::Av1 => sys::str::AMFVideoDecoderHW_AV1,
    };
    try_create_component(context, component_id)
}

/// 指定のコンポーネント ID のコンポーネントを作成できるかを返す
fn try_create_component(context: &Context, component_id: &str) -> bool {
    let lib = AmfLibrary::instance();
    // 対応判定にはコンポーネントの作成だけで十分なため、Init は呼ばない。
    lib.create_component(context, component_id).is_ok()
}
