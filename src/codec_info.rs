//! コーデック情報の照会

use std::ptr;

use crate::AmfLibrary;
use crate::sys::{self, AMF_RESULT, AMFComponent, AMFContext};

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
/// AMF ランタイムがロードできない場合は全コーデック非対応を返す。
pub fn supported_codecs() -> Vec<CodecInfo> {
    let codecs = VideoCodecType::all();

    let probe = match ProbeContext::new() {
        Some(p) => p,
        None => {
            return codecs.iter().map(|&codec| not_supported(codec)).collect();
        }
    };

    codecs
        .iter()
        .map(|&codec| {
            let enc_supported = probe.try_create_encoder(codec);
            let dec_supported = probe.try_create_decoder(codec);

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

/// コーデック対応判定用の内部コンテキスト
///
/// Drop で AMFContext を安全に解放する。
struct ProbeContext {
    lib: AmfLibrary,
    context: *mut AMFContext,
}

impl ProbeContext {
    /// AMF ランタイムをロードして判定用コンテキストを作成する
    ///
    /// ロードまたは Vulkan 初期化に失敗した場合は None を返す。
    fn new() -> Option<Self> {
        let lib = AmfLibrary::load().ok()?;
        let context = lib.create_context().ok()?;
        AmfLibrary::init_vulkan(context).ok()?;
        Some(Self { lib, context })
    }

    /// エンコーダーの CreateComponent を試みて対応状況を返す
    fn try_create_encoder(&self, codec: VideoCodecType) -> bool {
        let component_id = match codec {
            VideoCodecType::H264 => sys::AMFVideoEncoderVCE_AVC,
            VideoCodecType::Hevc => sys::AMFVideoEncoder_HEVC,
            VideoCodecType::Av1 => sys::AMFVideoEncoder_AV1,
        };
        self.try_create_component(component_id)
    }

    /// デコーダーの CreateComponent を試みて対応状況を返す
    fn try_create_decoder(&self, codec: VideoCodecType) -> bool {
        let component_id = match codec {
            VideoCodecType::H264 => sys::AMFVideoDecoderUVD_H264_AVC,
            VideoCodecType::Hevc => sys::AMFVideoDecoderHW_H265_HEVC,
            VideoCodecType::Av1 => sys::AMFVideoDecoderHW_AV1,
        };
        self.try_create_component(component_id)
    }

    /// 指定のコンポーネント ID で CreateComponent を試みる
    ///
    /// 成功した場合はコンポーネントを即座に解放して true を返す。
    fn try_create_component(&self, component_id: &str) -> bool {
        let component_id_w = sys::to_wstring(component_id);
        let mut component: *mut AMFComponent = ptr::null_mut();

        let result = unsafe {
            let vtbl = &*(*self.lib.factory()).pVtbl;
            vtbl.CreateComponent.unwrap()(
                self.lib.factory(),
                self.context,
                component_id_w.as_ptr(),
                &mut component,
            )
        };

        if result != AMF_RESULT::AMF_OK || component.is_null() {
            return false;
        }

        // エンコーダーは Init なしでは Terminate できないためスキップする。
        // デコーダーは Init(NV12, 0, 0) で初期化できるが、probe 目的では不要。
        // CreateComponent が成功すればそのコーデックは対応している。
        unsafe {
            let vtbl = &*(*component).pVtbl;
            // Init を呼ばずに作成したコンポーネントでも Release は必要
            vtbl.Release.unwrap()(component);
        }

        true
    }
}

impl Drop for ProbeContext {
    fn drop(&mut self) {
        // Drop 内の panic は二重 panic で abort になるため、
        // vtable の関数ポインタが欠けている場合は握りつぶす
        unsafe {
            let vtbl = &*(*self.context).pVtbl;
            if let Some(terminate) = vtbl.Terminate {
                let _ = terminate(self.context);
            }
            if let Some(release) = vtbl.Release {
                release(self.context);
            }
        }
    }
}
