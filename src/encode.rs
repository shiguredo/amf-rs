//! AMF ハードウェアエンコーダー
//!
//! AMD GPU を使ったハードウェアビデオエンコードを提供する。
//! H.264/AVC、H.265/HEVC、AV1 コーデックに対応する。

use std::collections::VecDeque;
use std::ptr;

use crate::AmfLibrary;
use crate::error::Error;
use crate::sys::{
    self, AMF_MEMORY_TYPE, AMF_PLANE_TYPE, AMF_RESULT, AMF_SECOND, AMF_SURFACE_FORMAT, AMFBuffer,
    AMFComponent, AMFContext, AMFData, AMFSurface, AMFVariantStruct, amf_int32, amf_int64, amf_pts,
};

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

/// エンコーダーの入力フレームフォーマット
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    /// Semi-Planar YUV 4:2:0 8bit
    Nv12,
    /// Planar YUV 4:2:0 8bit (Y+V+U)
    Yv12,
    /// Planar YUV 4:2:0 8bit (I420)
    I420,
    /// Packed BGRA 8bit
    Bgra,
    /// Packed ARGB 8bit
    Argb,
    /// Packed RGBA 8bit
    Rgba,
    /// Packed YUV 4:2:2 8bit (YUY2)
    Yuy2,
    /// Packed YUV 4:2:2 8bit (UYVY)
    Uyvy,
    /// Semi-Planar YUV 4:2:0 10bit
    P010,
    /// Semi-Planar YUV 4:2:0 12bit (16bit 格納)
    P012,
    /// Semi-Planar YUV 4:2:0 16bit
    P016,
    /// Packed YUV 4:2:2 10bit (16bit 格納)
    Y210,
    /// Packed YUV 4:4:4 8bit
    Ayuv,
    /// Packed YUV 4:4:4 10bit
    Y410,
    /// Packed YUV 4:4:4 16bit
    Y416,
}

impl FrameFormat {
    /// AMF_SURFACE_FORMAT に変換する
    fn to_amf(self) -> AMF_SURFACE_FORMAT {
        match self {
            FrameFormat::Nv12 => AMF_SURFACE_FORMAT::AMF_SURFACE_NV12,
            FrameFormat::Yv12 => AMF_SURFACE_FORMAT::AMF_SURFACE_YV12,
            FrameFormat::I420 => AMF_SURFACE_FORMAT::AMF_SURFACE_YUV420P,
            FrameFormat::Bgra => AMF_SURFACE_FORMAT::AMF_SURFACE_BGRA,
            FrameFormat::Argb => AMF_SURFACE_FORMAT::AMF_SURFACE_ARGB,
            FrameFormat::Rgba => AMF_SURFACE_FORMAT::AMF_SURFACE_RGBA,
            FrameFormat::Yuy2 => AMF_SURFACE_FORMAT::AMF_SURFACE_YUY2,
            FrameFormat::Uyvy => AMF_SURFACE_FORMAT::AMF_SURFACE_UYVY,
            FrameFormat::P010 => AMF_SURFACE_FORMAT::AMF_SURFACE_P010,
            FrameFormat::P012 => AMF_SURFACE_FORMAT::AMF_SURFACE_P012,
            FrameFormat::P016 => AMF_SURFACE_FORMAT::AMF_SURFACE_P016,
            FrameFormat::Y210 => AMF_SURFACE_FORMAT::AMF_SURFACE_Y210,
            FrameFormat::Ayuv => AMF_SURFACE_FORMAT::AMF_SURFACE_AYUV,
            FrameFormat::Y410 => AMF_SURFACE_FORMAT::AMF_SURFACE_Y410,
            FrameFormat::Y416 => AMF_SURFACE_FORMAT::AMF_SURFACE_Y416,
        }
    }

    /// 指定解像度でのフレームサイズ（バイト数）を計算する
    pub fn frame_size(&self, width: usize, height: usize) -> usize {
        match self {
            // YUV 4:2:0 8bit 系: width * height * 3 / 2
            FrameFormat::Nv12 | FrameFormat::Yv12 | FrameFormat::I420 => width * height * 3 / 2,
            // Packed 32bit 系: width * height * 4
            FrameFormat::Bgra
            | FrameFormat::Argb
            | FrameFormat::Rgba
            | FrameFormat::Ayuv
            | FrameFormat::Y410 => width * height * 4,
            // Packed YUV 4:2:2 8bit 系: width * height * 2
            FrameFormat::Yuy2 | FrameFormat::Uyvy => width * height * 2,
            // Semi-Planar YUV 4:2:0 16bit 系: width * height * 3 (16bit per component)
            FrameFormat::P010 | FrameFormat::P012 | FrameFormat::P016 => width * height * 3,
            // Packed YUV 4:2:2 10bit (16bit 格納): width * height * 4
            FrameFormat::Y210 => width * height * 4,
            // Packed YUV 4:4:4 16bit: width * height * 8
            FrameFormat::Y416 => width * height * 8,
        }
    }
}

/// レート制御モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateControlMode {
    /// 固定 QP
    Cqp,
    /// 固定ビットレート
    Cbr,
    /// ピーク制約付き可変ビットレート
    Vbr,
    /// レイテンシ制約付き可変ビットレート
    LatencyConstrainedVbr,
    /// 品質 VBR
    QualityVbr,
    /// 高品質 VBR
    HighQualityVbr,
    /// 高品質 CBR
    HighQualityCbr,
}

/// ピクチャタイプ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureType {
    /// IDR フレーム
    Idr,
    /// I フレーム
    I,
    /// P フレーム
    P,
    /// B フレーム
    B,
    /// 不明
    Unknown,
}

/// H.264 プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Profile {
    Baseline,
    Main,
    High,
    ConstrainedBaseline,
    ConstrainedHigh,
}

/// H.264 エンコーダー固有設定
#[derive(Debug, Clone)]
pub struct H264EncoderConfig {
    pub profile: Option<H264Profile>,
}

/// HEVC プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcProfile {
    Main,
    Main10,
}

/// HEVC エンコーダー固有設定
#[derive(Debug, Clone)]
pub struct HevcEncoderConfig {
    pub profile: Option<HevcProfile>,
}

/// AV1 プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Av1Profile {
    Main,
}

/// AV1 エンコーダー固有設定
#[derive(Debug, Clone)]
pub struct Av1EncoderConfig {
    pub profile: Option<Av1Profile>,
}

/// コーデック設定
#[derive(Debug, Clone)]
pub enum CodecConfig {
    H264(H264EncoderConfig),
    Hevc(HevcEncoderConfig),
    Av1(Av1EncoderConfig),
}

/// エンコーダー設定
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub codec: CodecConfig,
    pub width: u32,
    pub height: u32,
    pub frame_format: FrameFormat,
    pub framerate_num: u32,
    pub framerate_den: u32,
    pub rate_control_mode: RateControlMode,
    pub target_kbps: Option<u32>,
    pub max_kbps: Option<u32>,
    pub qpi: Option<u16>,
    pub qpp: Option<u16>,
    pub qpb: Option<u16>,
    pub gop_pic_size: Option<u16>,
}

impl EncoderConfig {
    pub fn new(
        codec: CodecConfig,
        width: u32,
        height: u32,
        frame_format: FrameFormat,
        framerate_num: u32,
        framerate_den: u32,
        rate_control_mode: RateControlMode,
    ) -> Self {
        Self {
            codec,
            width,
            height,
            frame_format,
            framerate_num,
            framerate_den,
            rate_control_mode,
            target_kbps: None,
            max_kbps: None,
            qpi: None,
            qpp: None,
            qpb: None,
            gop_pic_size: None,
        }
    }
}

/// エンコードオプション（フレームごと）
#[derive(Default)]
pub struct EncodeOptions {
    /// 強制するフレームタイプ (0 = 自動)
    pub frame_type: u16,
}

/// エンコード済みフレーム
pub struct EncodedFrame {
    data: Vec<u8>,
    pts: i64,
    picture_type: PictureType,
}

impl EncodedFrame {
    /// エンコード済みビットストリームデータ
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// フレームデータの所有権を取得する
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// PTS (100 ナノ秒単位)
    pub fn pts(&self) -> i64 {
        self.pts
    }

    /// ピクチャタイプ
    pub fn picture_type(&self) -> PictureType {
        self.picture_type
    }
}

// ---------------------------------------------------------------------------
// frame_type 定数 (vpl-rs 互換)
//
// AMF SDK の定義 (AMF_VIDEO_ENCODER_PICTURE_TYPE_ENUM 等) とは独立した
// フレームタイプフラグ。vpl-rs と共通のインターフェースを提供するために定義している。
// AMF の force_picture_type プロパティへの変換は Encoder::force_picture_type() で行う。
// ---------------------------------------------------------------------------
pub mod frame_type {
    pub const UNKNOWN: u16 = 0;
    pub const I: u16 = 0x0001;
    pub const P: u16 = 0x0002;
    pub const B: u16 = 0x0004;
    pub const IDR: u16 = 0x0020;
    pub const REF: u16 = 0x0040;
}

// ---------------------------------------------------------------------------
// エンコーダー実装
// ---------------------------------------------------------------------------

/// AMF ハードウェアエンコーダー
pub struct Encoder {
    // ライブラリハンドルを保持して Drop 順序で dlclose が最後に呼ばれることを保証する
    _lib: AmfLibrary,
    context: *mut AMFContext,
    component: *mut AMFComponent,
    surface_format: AMF_SURFACE_FORMAT,
    frame_format: FrameFormat,
    width: i32,
    height: i32,
    encoded_frames: VecDeque<EncodedFrame>,
    frame_count: u64,
    framerate_num: u64,
    framerate_den: u64,
    codec_config: CodecConfig,
}

// 安全性: AMF のコンポーネントはスレッドセーフではない (Sync は実装しない)。
// Send のみ実装することで、生成スレッドから使用スレッドへの所有権移動を許可する。
// 同時アクセスはコンパイラが Sync の欠如によって防止する。
unsafe impl Send for Encoder {}

impl Encoder {
    /// エンコーダーを作成して初期化する
    pub fn new(config: EncoderConfig) -> Result<Self, Error> {
        // 入力パラメータのバリデーション
        if config.width == 0 || config.height == 0 {
            return Err(Error::new_custom(
                "Encoder::new",
                &format!(
                    "invalid resolution: {}x{} (must be non-zero)",
                    config.width, config.height
                ),
            ));
        }
        if config.width > i32::MAX as u32 || config.height > i32::MAX as u32 {
            return Err(Error::new_custom(
                "Encoder::new",
                &format!(
                    "invalid resolution: {}x{} (exceeds i32::MAX)",
                    config.width, config.height
                ),
            ));
        }
        if !config.width.is_multiple_of(2) || !config.height.is_multiple_of(2) {
            return Err(Error::new_custom(
                "Encoder::new",
                &format!(
                    "invalid resolution: {}x{} (must be even)",
                    config.width, config.height
                ),
            ));
        }
        if config.framerate_num == 0 || config.framerate_den == 0 {
            return Err(Error::new_custom(
                "Encoder::new",
                &format!(
                    "invalid framerate: {}/{} (must be non-zero)",
                    config.framerate_num, config.framerate_den
                ),
            ));
        }

        let lib = AmfLibrary::load()?;
        let context = lib.create_context()?;

        // Linux では Vulkan/OpenCL でコンテキストを初期化する
        AmfLibrary::init_vulkan(context)?;

        // コーデック固有のコンポーネント ID を選択する
        let component_id = match &config.codec {
            CodecConfig::H264(_) => sys::AMFVideoEncoderVCE_AVC,
            CodecConfig::Hevc(_) => sys::AMFVideoEncoder_HEVC,
            CodecConfig::Av1(_) => sys::AMFVideoEncoder_AV1,
        };
        let component_id_w = sys::to_wstring(component_id);

        // コンポーネントを作成する
        let mut component: *mut AMFComponent = ptr::null_mut();
        let result = unsafe {
            let vtbl = &*(*lib.factory()).pVtbl;
            vtbl.CreateComponent.unwrap()(
                lib.factory(),
                context,
                component_id_w.as_ptr(),
                &mut component,
            )
        };
        Error::check(result, "AMFFactory::CreateComponent")?;

        if component.is_null() {
            return Err(Error::new_custom(
                "Encoder::new",
                "CreateComponent returned null",
            ));
        }

        let surface_format = config.frame_format.to_amf();

        // プロパティを設定する
        Self::set_properties(component, &config)?;

        // エンコーダーを初期化する
        let result = unsafe {
            let vtbl = &*(*component).pVtbl;
            vtbl.Init.unwrap()(
                component,
                surface_format,
                config.width as amf_int32,
                config.height as amf_int32,
            )
        };
        Error::check(result, "AMFComponent::Init")?;

        Ok(Self {
            _lib: lib,
            context,
            component,
            surface_format,
            frame_format: config.frame_format,
            width: config.width as i32,
            height: config.height as i32,
            encoded_frames: VecDeque::new(),
            frame_count: 0,
            framerate_num: config.framerate_num as u64,
            framerate_den: config.framerate_den as u64,
            codec_config: config.codec,
        })
    }

    /// エンコーダーにプロパティを設定する
    fn set_properties(component: *mut AMFComponent, config: &EncoderConfig) -> Result<(), Error> {
        match &config.codec {
            CodecConfig::H264(h264) => {
                Self::set_h264_properties(component, config, h264)?;
            }
            CodecConfig::Hevc(hevc) => {
                Self::set_hevc_properties(component, config, hevc)?;
            }
            CodecConfig::Av1(av1) => {
                Self::set_av1_properties(component, config, av1)?;
            }
        }
        Ok(())
    }

    /// H.264 固有のプロパティを設定する
    fn set_h264_properties(
        component: *mut AMFComponent,
        config: &EncoderConfig,
        h264: &H264EncoderConfig,
    ) -> Result<(), Error> {
        // Usage: トランスコーディング
        set_property_int64(component, sys::AMF_VIDEO_ENCODER_USAGE, 0)?;

        // Profile
        if let Some(profile) = h264.profile {
            let profile_val: amf_int64 = match profile {
                H264Profile::Baseline => 66,
                H264Profile::Main => 77,
                H264Profile::High => 100,
                H264Profile::ConstrainedBaseline => 256,
                H264Profile::ConstrainedHigh => 257,
            };
            set_property_int64(component, sys::AMF_VIDEO_ENCODER_PROFILE, profile_val)?;
        }

        // Frame size
        set_property_size(
            component,
            sys::AMF_VIDEO_ENCODER_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
        )?;

        // Frame rate
        set_property_rate(
            component,
            sys::AMF_VIDEO_ENCODER_FRAMERATE,
            config.framerate_num,
            config.framerate_den,
        )?;

        // Rate control
        // AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM の値
        let rc_method: amf_int64 = match config.rate_control_mode {
            RateControlMode::Cqp => 0, // AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_CONSTANT_QP
            RateControlMode::Cbr => 1, // AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_CBR
            RateControlMode::Vbr => 2, // AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR
            RateControlMode::LatencyConstrainedVbr => 3, // AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR
            RateControlMode::QualityVbr => 4, // AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_QUALITY_VBR
            RateControlMode::HighQualityVbr => 5, // AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR
            RateControlMode::HighQualityCbr => 6, // AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR
        };
        set_property_int64(
            component,
            sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD,
            rc_method,
        )?;

        // Bitrate
        if let Some(target_kbps) = config.target_kbps {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_TARGET_BITRATE,
                target_kbps as amf_int64 * 1000,
            )?;
        }
        if let Some(max_kbps) = config.max_kbps {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_PEAK_BITRATE,
                max_kbps as amf_int64 * 1000,
            )?;
        }

        // QP
        if let Some(qpi) = config.qpi {
            set_property_int64(component, sys::AMF_VIDEO_ENCODER_QP_I, qpi as amf_int64)?;
        }
        if let Some(qpp) = config.qpp {
            set_property_int64(component, sys::AMF_VIDEO_ENCODER_QP_P, qpp as amf_int64)?;
        }
        if let Some(qpb) = config.qpb {
            set_property_int64(component, sys::AMF_VIDEO_ENCODER_QP_B, qpb as amf_int64)?;
        }

        // GOP / IDR
        if let Some(gop) = config.gop_pic_size {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_IDR_PERIOD,
                gop as amf_int64,
            )?;
        }

        Ok(())
    }

    /// HEVC 固有のプロパティを設定する
    fn set_hevc_properties(
        component: *mut AMFComponent,
        config: &EncoderConfig,
        hevc: &HevcEncoderConfig,
    ) -> Result<(), Error> {
        set_property_int64(component, sys::AMF_VIDEO_ENCODER_HEVC_USAGE, 0)?;

        if let Some(profile) = hevc.profile {
            let profile_val: amf_int64 = match profile {
                HevcProfile::Main => 1,
                HevcProfile::Main10 => 2,
            };
            set_property_int64(component, sys::AMF_VIDEO_ENCODER_HEVC_PROFILE, profile_val)?;
        }

        set_property_size(
            component,
            sys::AMF_VIDEO_ENCODER_HEVC_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
        )?;

        set_property_rate(
            component,
            sys::AMF_VIDEO_ENCODER_HEVC_FRAMERATE,
            config.framerate_num,
            config.framerate_den,
        )?;

        // HEVC のレート制御は列挙値の順序が H.264 と異なる
        // AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM の値
        let rc_method: amf_int64 = match config.rate_control_mode {
            RateControlMode::Cqp => 0, // AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_CONSTANT_QP
            RateControlMode::LatencyConstrainedVbr => 1, // AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR
            RateControlMode::Vbr => 2, // AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR
            RateControlMode::Cbr => 3, // AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_CBR
            RateControlMode::QualityVbr => 4, // AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_QUALITY_VBR
            RateControlMode::HighQualityVbr => 5, // AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR
            RateControlMode::HighQualityCbr => 6, // AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR
        };
        set_property_int64(
            component,
            sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD,
            rc_method,
        )?;

        if let Some(target_kbps) = config.target_kbps {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_HEVC_TARGET_BITRATE,
                target_kbps as amf_int64 * 1000,
            )?;
        }
        if let Some(max_kbps) = config.max_kbps {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_HEVC_PEAK_BITRATE,
                max_kbps as amf_int64 * 1000,
            )?;
        }

        if let Some(qpi) = config.qpi {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_HEVC_QP_I,
                qpi as amf_int64,
            )?;
        }
        if let Some(qpp) = config.qpp {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_HEVC_QP_P,
                qpp as amf_int64,
            )?;
        }
        // AMF HEVC エンコーダーには B フレーム用の QP プロパティが存在しないため
        // qpb は設定しない (H.264 の AMF_VIDEO_ENCODER_QP_B に相当するものがない)

        if let Some(gop) = config.gop_pic_size {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_HEVC_GOP_SIZE,
                gop as amf_int64,
            )?;
        }

        Ok(())
    }

    /// AV1 固有のプロパティを設定する
    fn set_av1_properties(
        component: *mut AMFComponent,
        config: &EncoderConfig,
        _av1: &Av1EncoderConfig,
    ) -> Result<(), Error> {
        set_property_int64(component, sys::AMF_VIDEO_ENCODER_AV1_USAGE, 0)?;

        set_property_size(
            component,
            sys::AMF_VIDEO_ENCODER_AV1_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
        )?;

        set_property_rate(
            component,
            sys::AMF_VIDEO_ENCODER_AV1_FRAMERATE,
            config.framerate_num,
            config.framerate_den,
        )?;

        // AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM の値
        // (HEVC と同じ順序)
        let rc_method: amf_int64 = match config.rate_control_mode {
            RateControlMode::Cqp => 0, // AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_CONSTANT_QP
            RateControlMode::LatencyConstrainedVbr => 1, // AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR
            RateControlMode::Vbr => 2, // AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR
            RateControlMode::Cbr => 3, // AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_CBR
            RateControlMode::QualityVbr => 4, // AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_QUALITY_VBR
            RateControlMode::HighQualityVbr => 5, // AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR
            RateControlMode::HighQualityCbr => 6, // AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR
        };
        set_property_int64(
            component,
            sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD,
            rc_method,
        )?;

        if let Some(target_kbps) = config.target_kbps {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_AV1_TARGET_BITRATE,
                target_kbps as amf_int64 * 1000,
            )?;
        }
        if let Some(max_kbps) = config.max_kbps {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_AV1_PEAK_BITRATE,
                max_kbps as amf_int64 * 1000,
            )?;
        }

        if let Some(qpi) = config.qpi {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTRA,
                qpi as amf_int64,
            )?;
        }
        if let Some(qpp) = config.qpp {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTER,
                qpp as amf_int64,
            )?;
        }

        if let Some(gop) = config.gop_pic_size {
            set_property_int64(
                component,
                sys::AMF_VIDEO_ENCODER_AV1_GOP_SIZE,
                gop as amf_int64,
            )?;
        }

        Ok(())
    }

    /// フレームをエンコードする
    pub fn encode(&mut self, frame_data: &[u8], options: &EncodeOptions) -> Result<(), Error> {
        let expected_size = self
            .frame_format
            .frame_size(self.width as usize, self.height as usize);
        if frame_data.len() != expected_size {
            return Err(Error::new_custom(
                "Encoder::encode",
                &format!(
                    "frame data size mismatch: expected {expected_size}, got {}",
                    frame_data.len()
                ),
            ));
        }

        log::debug!("Encoder::encode: AllocSurface");
        // Surface を確保する
        let mut surface: *mut AMFSurface = ptr::null_mut();
        let result = unsafe {
            let vtbl = &*(*self.context).pVtbl;
            vtbl.AllocSurface.unwrap()(
                self.context,
                AMF_MEMORY_TYPE::AMF_MEMORY_HOST,
                self.surface_format,
                self.width,
                self.height,
                &mut surface,
            )
        };
        Error::check(result, "AMFContext::AllocSurface")?;

        if surface.is_null() {
            return Err(Error::new_custom(
                "Encoder::encode",
                "AllocSurface returned null",
            ));
        }

        // フレームデータを Surface にコピーする
        self.copy_frame_to_surface(surface, frame_data)?;

        // PTS を設定する
        let pts = (self.frame_count * self.framerate_den * AMF_SECOND as u64 / self.framerate_num)
            as amf_pts;
        unsafe {
            let vtbl = &*(*surface).pVtbl;
            vtbl.SetPts.unwrap()(surface, pts);
        }

        // フレームタイプの強制
        if options.frame_type & frame_type::IDR != 0 {
            self.force_picture_type(surface)?;
        }

        log::debug!("Encoder::encode: SubmitInput");
        // SubmitInput (INPUT_FULL の場合は出力をポーリングしてリトライする)
        let max_retries = 100;
        let mut submitted = false;
        for retry in 0..max_retries {
            let result = unsafe {
                let vtbl = &*(*self.component).pVtbl;
                vtbl.SubmitInput.unwrap()(self.component, surface as *mut AMFData)
            };

            if result == AMF_RESULT::AMF_OK {
                submitted = true;
                break;
            }
            if result == AMF_RESULT::AMF_INPUT_FULL || result == AMF_RESULT::AMF_REPEAT {
                if retry > 0 && retry % 10 == 0 {
                    log::debug!("Encoder::SubmitInput: retry={retry}, result={result:?}");
                }
                self.poll_output()?;
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            // その他のエラー: Surface を解放してエラーを返す
            unsafe {
                let vtbl = &*(*surface).pVtbl;
                vtbl.Release.unwrap()(surface);
            }
            return Error::check(result, "AMFComponent::SubmitInput");
        }
        if !submitted {
            unsafe {
                let vtbl = &*(*surface).pVtbl;
                vtbl.Release.unwrap()(surface);
            }
            return Err(Error::new_custom(
                "Encoder::encode",
                "SubmitInput retry limit exceeded",
            ));
        }

        self.frame_count += 1;

        log::debug!("Encoder::encode: poll_output");
        // 出力をポーリングする
        self.poll_output()?;

        Ok(())
    }

    /// フレームデータを AMFSurface にコピーする
    fn copy_frame_to_surface(
        &self,
        surface: *mut AMFSurface,
        frame_data: &[u8],
    ) -> Result<(), Error> {
        match self.frame_format {
            FrameFormat::Nv12 | FrameFormat::P010 | FrameFormat::P012 | FrameFormat::P016 => {
                // Semi-Planar YUV 4:2:0: Y プレーン + UV インターリーブプレーン
                // NV12 は 8bit (1 バイト/サンプル)、P010/P012/P016 は 16bit (2 バイト/サンプル)
                let bytes_per_sample: usize = match self.frame_format {
                    FrameFormat::Nv12 => 1,
                    _ => 2,
                };
                let width = self.width as usize;
                let row_bytes = width * bytes_per_sample;

                // Y プレーン
                let y_plane = unsafe {
                    let vtbl = &*(*surface).pVtbl;
                    vtbl.GetPlane.unwrap()(surface, AMF_PLANE_TYPE::AMF_PLANE_Y)
                };
                if y_plane.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "failed to get Y plane",
                    ));
                }
                let y_native = unsafe {
                    let vtbl = &*(*y_plane).pVtbl;
                    vtbl.GetNative.unwrap()(y_plane) as *mut u8
                };
                if y_native.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "Y plane native pointer is null",
                    ));
                }
                let y_hpitch = unsafe {
                    let vtbl = &*(*y_plane).pVtbl;
                    vtbl.GetHPitch.unwrap()(y_plane) as usize
                };
                if y_hpitch < row_bytes {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "Y plane hpitch is smaller than row bytes",
                    ));
                }

                let height = self.height as usize;
                // Y プレーン: height 行 * row_bytes バイト
                let y_required = height * row_bytes;
                if y_required > frame_data.len() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "frame data too small for Y plane",
                    ));
                }

                for row in 0..height {
                    unsafe {
                        ptr::copy_nonoverlapping(
                            frame_data.as_ptr().add(row * row_bytes),
                            y_native.add(row * y_hpitch),
                            row_bytes,
                        );
                    }
                }

                // UV プレーン
                let uv_plane = unsafe {
                    let vtbl = &*(*surface).pVtbl;
                    vtbl.GetPlane.unwrap()(surface, AMF_PLANE_TYPE::AMF_PLANE_UV)
                };
                if uv_plane.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "failed to get UV plane",
                    ));
                }
                let uv_native = unsafe {
                    let vtbl = &*(*uv_plane).pVtbl;
                    vtbl.GetNative.unwrap()(uv_plane) as *mut u8
                };
                if uv_native.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "UV plane native pointer is null",
                    ));
                }
                let uv_hpitch = unsafe {
                    let vtbl = &*(*uv_plane).pVtbl;
                    vtbl.GetHPitch.unwrap()(uv_plane) as usize
                };
                if uv_hpitch < row_bytes {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "UV plane hpitch is smaller than row bytes",
                    ));
                }

                let uv_height = height / 2;
                let y_data_size = row_bytes * height;
                // UV プレーン: uv_height 行 * row_bytes バイト
                let uv_required = y_data_size + uv_height * row_bytes;
                if uv_required > frame_data.len() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "frame data too small for UV plane",
                    ));
                }

                for row in 0..uv_height {
                    unsafe {
                        ptr::copy_nonoverlapping(
                            frame_data.as_ptr().add(y_data_size + row * row_bytes),
                            uv_native.add(row * uv_hpitch),
                            row_bytes,
                        );
                    }
                }
            }
            FrameFormat::I420 | FrameFormat::Yv12 => {
                // Planar YUV 4:2:0: Y/U/V (I420) または Y/V/U (YV12) の 3 プレーン
                let width = self.width as usize;
                let height = self.height as usize;
                let y_size = width * height;
                let uv_plane_size = (width / 2) * (height / 2);

                // Y プレーン
                let y_plane = unsafe {
                    let vtbl = &*(*surface).pVtbl;
                    vtbl.GetPlane.unwrap()(surface, AMF_PLANE_TYPE::AMF_PLANE_Y)
                };
                if y_plane.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "failed to get Y plane",
                    ));
                }
                let y_native = unsafe {
                    let vtbl = &*(*y_plane).pVtbl;
                    vtbl.GetNative.unwrap()(y_plane) as *mut u8
                };
                if y_native.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "Y plane native pointer is null",
                    ));
                }
                let y_hpitch = unsafe {
                    let vtbl = &*(*y_plane).pVtbl;
                    vtbl.GetHPitch.unwrap()(y_plane) as usize
                };
                if y_hpitch < width {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "Y plane hpitch is smaller than width",
                    ));
                }
                // Y + U + V の合計サイズを事前検証する
                let total_required = y_size + uv_plane_size * 2;
                if total_required > frame_data.len() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "frame data too small for I420/YV12",
                    ));
                }

                for row in 0..height {
                    unsafe {
                        ptr::copy_nonoverlapping(
                            frame_data.as_ptr().add(row * width),
                            y_native.add(row * y_hpitch),
                            width,
                        );
                    }
                }

                // U プレーン
                let u_plane = unsafe {
                    let vtbl = &*(*surface).pVtbl;
                    vtbl.GetPlane.unwrap()(surface, AMF_PLANE_TYPE::AMF_PLANE_U)
                };
                if u_plane.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "failed to get U plane",
                    ));
                }
                let u_native = unsafe {
                    let vtbl = &*(*u_plane).pVtbl;
                    vtbl.GetNative.unwrap()(u_plane) as *mut u8
                };
                if u_native.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "U plane native pointer is null",
                    ));
                }
                let u_hpitch = unsafe {
                    let vtbl = &*(*u_plane).pVtbl;
                    vtbl.GetHPitch.unwrap()(u_plane) as usize
                };
                let uv_width = width / 2;
                let uv_height = height / 2;
                if u_hpitch < uv_width {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "U plane hpitch is smaller than uv_width",
                    ));
                }

                // I420: Y+U+V, YV12: Y+V+U
                // AMF はフォーマットに応じてプレーン順を管理するため、
                // GetPlane(AMF_PLANE_U/V) で正しいプレーンが返る
                let first_uv_offset = y_size;
                let second_uv_offset = y_size + uv_plane_size;

                for row in 0..uv_height {
                    unsafe {
                        ptr::copy_nonoverlapping(
                            frame_data.as_ptr().add(first_uv_offset + row * uv_width),
                            u_native.add(row * u_hpitch),
                            uv_width,
                        );
                    }
                }

                // V プレーン
                let v_plane = unsafe {
                    let vtbl = &*(*surface).pVtbl;
                    vtbl.GetPlane.unwrap()(surface, AMF_PLANE_TYPE::AMF_PLANE_V)
                };
                if v_plane.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "failed to get V plane",
                    ));
                }
                let v_native = unsafe {
                    let vtbl = &*(*v_plane).pVtbl;
                    vtbl.GetNative.unwrap()(v_plane) as *mut u8
                };
                if v_native.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "V plane native pointer is null",
                    ));
                }
                let v_hpitch = unsafe {
                    let vtbl = &*(*v_plane).pVtbl;
                    vtbl.GetHPitch.unwrap()(v_plane) as usize
                };
                if v_hpitch < uv_width {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "V plane hpitch is smaller than uv_width",
                    ));
                }

                for row in 0..uv_height {
                    unsafe {
                        ptr::copy_nonoverlapping(
                            frame_data.as_ptr().add(second_uv_offset + row * uv_width),
                            v_native.add(row * v_hpitch),
                            uv_width,
                        );
                    }
                }
            }
            _ => {
                // Packed フォーマット: 単一プレーンにコピーする
                let plane = unsafe {
                    let vtbl = &*(*surface).pVtbl;
                    vtbl.GetPlaneAt.unwrap()(surface, 0)
                };
                if plane.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "failed to get plane",
                    ));
                }
                let native = unsafe {
                    let vtbl = &*(*plane).pVtbl;
                    vtbl.GetNative.unwrap()(plane) as *mut u8
                };
                if native.is_null() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "plane native pointer is null",
                    ));
                }
                let hpitch = unsafe {
                    let vtbl = &*(*plane).pVtbl;
                    vtbl.GetHPitch.unwrap()(plane) as usize
                };
                let row_bytes = self.width as usize
                    * match self.frame_format {
                        // Packed 32bit 系
                        FrameFormat::Bgra
                        | FrameFormat::Argb
                        | FrameFormat::Rgba
                        | FrameFormat::Ayuv
                        | FrameFormat::Y410
                        | FrameFormat::Y210 => 4,
                        // Packed 64bit 系
                        FrameFormat::Y416 => 8,
                        // Packed YUV 4:2:2 8bit 系
                        FrameFormat::Yuy2 | FrameFormat::Uyvy => 2,
                        _ => unreachable!(),
                    };
                if hpitch < row_bytes {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "plane hpitch is smaller than row bytes",
                    ));
                }
                let height = self.height as usize;
                let total_required = height * row_bytes;
                if total_required > frame_data.len() {
                    return Err(Error::new_custom(
                        "copy_frame_to_surface",
                        "frame data too small for packed format",
                    ));
                }
                for row in 0..height {
                    unsafe {
                        ptr::copy_nonoverlapping(
                            frame_data.as_ptr().add(row * row_bytes),
                            native.add(row * hpitch),
                            row_bytes,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// IDR フレームを強制する
    fn force_picture_type(&self, surface: *mut AMFSurface) -> Result<(), Error> {
        let prop_name = match &self.codec_config {
            CodecConfig::H264(_) => sys::AMF_VIDEO_ENCODER_FORCE_PICTURE_TYPE,
            CodecConfig::Hevc(_) => sys::AMF_VIDEO_ENCODER_HEVC_FORCE_PICTURE_TYPE,
            CodecConfig::Av1(_) => sys::AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE,
        };
        // IDR = 2 (H.264), IDR = 2 (HEVC), KEY = 1 (AV1)
        let force_type: amf_int64 = match &self.codec_config {
            CodecConfig::H264(_) => 2, // AMF_VIDEO_ENCODER_PICTURE_TYPE_IDR
            CodecConfig::Hevc(_) => 2, // AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_IDR
            CodecConfig::Av1(_) => 1,  // AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE_KEY
        };
        let name_w = sys::to_wstring(prop_name);
        let var = AMFVariantStruct::from_int64(force_type);
        let result = unsafe {
            let vtbl = &*(*surface).pVtbl;
            vtbl.SetProperty.unwrap()(surface, name_w.as_ptr(), var)
        };
        Error::check(result, "AMFSurface::SetProperty(ForcePictureType)")
    }

    /// AMFData から EncodedFrame を抽出してキューに追加する
    fn extract_encoded_output(&mut self, data: *mut AMFData) -> Result<(), Error> {
        let buffer = data as *mut AMFBuffer;
        let buf_size = unsafe {
            let vtbl = &*(*buffer).pVtbl;
            vtbl.GetSize.unwrap()(buffer)
        };
        if buf_size == 0 {
            unsafe {
                let vtbl = &*(*buffer).pVtbl;
                vtbl.Release.unwrap()(buffer);
            }
            return Err(Error::new_custom(
                "extract_encoded_output",
                "buffer size is 0",
            ));
        }
        let buf_native = unsafe {
            let vtbl = &*(*buffer).pVtbl;
            vtbl.GetNative.unwrap()(buffer) as *const u8
        };
        if buf_native.is_null() {
            unsafe {
                let vtbl = &*(*buffer).pVtbl;
                vtbl.Release.unwrap()(buffer);
            }
            return Err(Error::new_custom(
                "extract_encoded_output",
                "buffer native pointer is null",
            ));
        }
        let pts = unsafe {
            let vtbl = &*(*buffer).pVtbl;
            vtbl.GetPts.unwrap()(buffer)
        };

        let frame_data = unsafe { std::slice::from_raw_parts(buf_native, buf_size) }.to_vec();
        let picture_type = self.get_output_picture_type(buffer);

        self.encoded_frames.push_back(EncodedFrame {
            data: frame_data,
            pts,
            picture_type,
        });

        unsafe {
            let vtbl = &*(*buffer).pVtbl;
            vtbl.Release.unwrap()(buffer);
        }
        Ok(())
    }

    /// 出力をポーリングしてエンコード済みフレームを取得する
    fn poll_output(&mut self) -> Result<(), Error> {
        loop {
            let mut data: *mut AMFData = ptr::null_mut();
            let result = unsafe {
                let vtbl = &*(*self.component).pVtbl;
                vtbl.QueryOutput.unwrap()(self.component, &mut data)
            };

            log::debug!("poll_output: QueryOutput result={result:?}");
            if result == AMF_RESULT::AMF_REPEAT || result == AMF_RESULT::AMF_EOF {
                break;
            }
            if result != AMF_RESULT::AMF_OK {
                log::debug!("poll_output: unexpected result, breaking");
                break;
            }
            if data.is_null() {
                break;
            }

            self.extract_encoded_output(data)?;
        }
        Ok(())
    }

    /// 出力バッファからピクチャタイプを取得する
    fn get_output_picture_type(&self, buffer: *mut AMFBuffer) -> PictureType {
        let prop_name = match &self.codec_config {
            CodecConfig::H264(_) => sys::AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE,
            CodecConfig::Hevc(_) => sys::AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE,
            CodecConfig::Av1(_) => sys::AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE,
        };
        let name_w = sys::to_wstring(prop_name);
        let mut var = AMFVariantStruct::empty();
        let result = unsafe {
            let vtbl = &*(*buffer).pVtbl;
            vtbl.GetProperty.unwrap()(buffer, name_w.as_ptr(), &mut var)
        };
        if result != AMF_RESULT::AMF_OK {
            return PictureType::Unknown;
        }

        let type_val = unsafe { var.__bindgen_anon_1.int64Value };
        match &self.codec_config {
            CodecConfig::H264(_) => match type_val {
                0 => PictureType::Idr,
                1 => PictureType::I,
                2 => PictureType::P,
                3 => PictureType::B,
                _ => PictureType::Unknown,
            },
            CodecConfig::Hevc(_) => match type_val {
                0 => PictureType::Idr,
                1 => PictureType::I,
                2 => PictureType::P,
                _ => PictureType::Unknown,
            },
            CodecConfig::Av1(_) => match type_val {
                0 => PictureType::Idr, // KEY
                _ => PictureType::P,
            },
        }
    }

    /// エンコード済みフレームを取り出す
    pub fn next_frame(&mut self) -> Option<EncodedFrame> {
        self.encoded_frames.pop_front()
    }

    /// エンコーダーをフラッシュして残りのフレームを取得する
    pub fn finish(&mut self) -> Result<(), Error> {
        // Drain を呼び出して残りのフレームを出力させる
        let result = unsafe {
            let vtbl = &*(*self.component).pVtbl;
            vtbl.Drain.unwrap()(self.component)
        };
        // AMF_INPUT_FULL は Drain 中に返ることがあるが無視する
        if result != AMF_RESULT::AMF_OK && result != AMF_RESULT::AMF_INPUT_FULL {
            Error::check(result, "AMFComponent::Drain")?;
        }

        // 残りの出力をポーリングする
        // AMF_REPEAT はエンコード中で出力が未準備の意味なのでリトライする。
        let max_repeat = 50;
        let mut repeat_count = 0;
        loop {
            let mut data: *mut AMFData = ptr::null_mut();
            let result = unsafe {
                let vtbl = &*(*self.component).pVtbl;
                vtbl.QueryOutput.unwrap()(self.component, &mut data)
            };
            if result == AMF_RESULT::AMF_EOF {
                break;
            }
            if result == AMF_RESULT::AMF_REPEAT {
                repeat_count += 1;
                if repeat_count > max_repeat {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            if result != AMF_RESULT::AMF_OK || data.is_null() {
                break;
            }

            repeat_count = 0;
            self.extract_encoded_output(data)?;
        }

        Ok(())
    }
}

// 安全性: new() が成功した場合のみ Self が構築されるため、
// component と context は常に有効なポインタであることが保証される。
impl Drop for Encoder {
    fn drop(&mut self) {
        // Drop 内の panic は二重 panic で abort になるため、
        // vtable の関数ポインタが欠けている場合は握りつぶす
        unsafe {
            let vtbl = &*(*self.component).pVtbl;
            if let Some(terminate) = vtbl.Terminate {
                let _ = terminate(self.component);
            }
            if let Some(release) = vtbl.Release {
                release(self.component);
            }

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

// ---------------------------------------------------------------------------
// ヘルパー関数
// ---------------------------------------------------------------------------

/// AMFComponent に Int64 プロパティを設定する
fn set_property_int64(
    component: *mut AMFComponent,
    name: &str,
    value: amf_int64,
) -> Result<(), Error> {
    let name_w = sys::to_wstring(name);
    let var = AMFVariantStruct::from_int64(value);
    let result = unsafe {
        let vtbl = &*(*component).pVtbl;
        vtbl.SetProperty.unwrap()(component, name_w.as_ptr(), var)
    };
    Error::check(result, format!("SetProperty({name})"))
}

/// AMFComponent に Size プロパティを設定する
fn set_property_size(
    component: *mut AMFComponent,
    name: &str,
    width: amf_int32,
    height: amf_int32,
) -> Result<(), Error> {
    let name_w = sys::to_wstring(name);
    let var = AMFVariantStruct::from_size(width, height);
    let result = unsafe {
        let vtbl = &*(*component).pVtbl;
        vtbl.SetProperty.unwrap()(component, name_w.as_ptr(), var)
    };
    Error::check(result, format!("SetProperty({name})"))
}

/// AMFComponent に Rate プロパティを設定する
fn set_property_rate(
    component: *mut AMFComponent,
    name: &str,
    num: u32,
    den: u32,
) -> Result<(), Error> {
    let name_w = sys::to_wstring(name);
    let var = AMFVariantStruct::from_rate(num, den);
    let result = unsafe {
        let vtbl = &*(*component).pVtbl;
        vtbl.SetProperty.unwrap()(component, name_w.as_ptr(), var)
    };
    Error::check(result, format!("SetProperty({name})"))
}
