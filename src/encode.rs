//! AMF ハードウェアエンコーダー
//!
//! AMD GPU を使ったハードウェアビデオエンコードを提供する。
//! H.264/AVC、H.265/HEVC、AV1 コーデックに対応する。

use std::collections::VecDeque;
use std::ptr;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::AmfLibrary;
use crate::amf::{Buffer, Component, Context, PropertyStorage, Surface};
use crate::error::Error;
use crate::sys::{
    self, AMF_MEMORY_TYPE, AMF_RESULT, AMF_SECOND, AMF_SURFACE_FORMAT, AMFBuffer, AMFData,
    AMFVariantStruct, amf_int32, amf_int64, amf_pts,
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
    ///
    /// オーバーフローした場合は `None` を返す。
    pub fn frame_size(&self, width: usize, height: usize) -> Option<usize> {
        let pixels = width.checked_mul(height)?;
        match self {
            // YUV 4:2:0 8bit 系: width * height * 3 / 2
            FrameFormat::Nv12 | FrameFormat::Yv12 | FrameFormat::I420 => {
                pixels.checked_mul(3).map(|v| v / 2)
            }
            // Packed 32bit 系: width * height * 4
            FrameFormat::Bgra
            | FrameFormat::Argb
            | FrameFormat::Rgba
            | FrameFormat::Ayuv
            | FrameFormat::Y410 => pixels.checked_mul(4),
            // Packed YUV 4:2:2 8bit 系: width * height * 2
            FrameFormat::Yuy2 | FrameFormat::Uyvy => pixels.checked_mul(2),
            // Semi-Planar YUV 4:2:0 16bit 系: width * height * 3 (16bit per component)
            FrameFormat::P010 | FrameFormat::P012 | FrameFormat::P016 => pixels.checked_mul(3),
            // Packed YUV 4:2:2 10bit (16bit 格納): width * height * 4
            FrameFormat::Y210 => pixels.checked_mul(4),
            // Packed YUV 4:4:4 16bit: width * height * 8
            FrameFormat::Y416 => pixels.checked_mul(8),
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
#[derive(Debug, Clone, Copy, Default)]
pub struct EncodeOptions {
    /// 強制するフレームタイプ (0 = 自動)
    pub frame_type: u16,
}

/// エンコーダー再設定パラメータ
///
/// `None` の項目は変更しない。
#[derive(Debug, Clone, Default)]
pub struct ReconfigureParams {
    pub framerate_num: Option<u32>,
    pub framerate_den: Option<u32>,
    pub target_kbps: Option<u32>,
    pub max_kbps: Option<u32>,
    pub qpi: Option<u16>,
    pub qpp: Option<u16>,
    pub qpb: Option<u16>,
    pub gop_pic_size: Option<u16>,
}

/// エンコード済みフレーム
#[derive(Debug)]
pub struct EncodedFrame<T> {
    buffer: Buffer,
    picture_type: PictureType,
    user_data: T,
}

impl<T> EncodedFrame<T> {
    /// エンコード済みビットストリームバッファ
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// ピクチャタイプ
    pub fn picture_type(&self) -> PictureType {
        self.picture_type
    }

    /// ユーザーデータ
    pub fn user_data(&self) -> &T {
        &self.user_data
    }

    /// ビットストリームバッファとユーザーデータの所有権を取得する
    pub fn into_parts(self) -> (Buffer, T) {
        (self.buffer, self.user_data)
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
// ハンドラートレイト
// ---------------------------------------------------------------------------

/// エンコード結果を通知するためのハンドラー
///
/// エンコード処理が完了するたびに [`EncodeHandler::on_encoded`] が呼ばれる。
pub trait EncodeHandler: Send + 'static {
    /// ユーザーデータ型
    type UserData: Send + 'static;
    /// エラー型
    type Error: From<crate::Error> + Send + 'static;
    /// エンコード完了時に呼ばれる
    fn on_encoded(&mut self, result: Result<EncodedFrame<Self::UserData>, Self::Error>);
}

/// `FnMut` クロージャを [`EncodeHandler`] にするラッパー
pub struct FnEncodeHandler<T, E = crate::Error> {
    f: Box<dyn FnMut(Result<EncodedFrame<T>, E>) + Send + 'static>,
}

impl<T, E> FnEncodeHandler<T, E> {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(Result<EncodedFrame<T>, E>) + Send + 'static,
    {
        Self { f: Box::new(f) }
    }
}

impl<T, E> EncodeHandler for FnEncodeHandler<T, E>
where
    T: Send + 'static,
    E: From<crate::Error> + Send + 'static,
{
    type UserData = T;
    type Error = E;
    fn on_encoded(&mut self, result: Result<EncodedFrame<T>, E>) {
        (self.f)(result);
    }
}

// ---------------------------------------------------------------------------
// ワーカースレッド用コマンド
// ---------------------------------------------------------------------------

enum WorkerCommand<T> {
    Submit(T),
    Finish(mpsc::SyncSender<()>),
}

// ---------------------------------------------------------------------------
// エンコーダー実装
// ---------------------------------------------------------------------------

/// AMF ハードウェアエンコーダー
pub struct Encoder<H: EncodeHandler> {
    component: Component,
    context: Context,
    surface_format: AMF_SURFACE_FORMAT,
    width: i32,
    height: i32,
    frame_count: u64,
    framerate_num: u64,
    framerate_den: u64,
    codec_config: CodecConfig,
    cmd_tx: Option<mpsc::Sender<WorkerCommand<H::UserData>>>,
    poll_thread: Option<JoinHandle<()>>,
}

impl<H: EncodeHandler> Encoder<H> {
    /// エンコーダーを作成して初期化する
    ///
    /// `handler` はエンコード完了時にワーカースレッドから呼び出される。
    pub fn new(config: EncoderConfig, handler: H) -> Result<Self, Error> {
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

        let lib = AmfLibrary::instance();
        let context = lib.create_context()?;

        // Linux では Vulkan/OpenCL でコンテキストを初期化する
        unsafe { context.init_vulkan(ptr::null_mut()) }?;

        // コーデック固有のコンポーネント ID を選択する
        let component_id = match &config.codec {
            CodecConfig::H264(_) => sys::str::AMFVideoEncoderVCE_AVC,
            CodecConfig::Hevc(_) => sys::str::AMFVideoEncoder_HEVC,
            CodecConfig::Av1(_) => sys::str::AMFVideoEncoder_AV1,
        };

        // コンポーネントを作成する
        let component = lib.create_component(&context, component_id)?;

        let surface_format = config.frame_format.to_amf();

        // プロパティを設定する
        let prop = component.property_storage()?;
        Self::set_properties(&prop, &config)?;

        // エンコーダーを初期化する
        let result = component.init(
            surface_format,
            config.width as amf_int32,
            config.height as amf_int32,
        );
        Error::check(result, "AMFComponent::Init")?;

        let codec_config = config.codec;

        // ワーカースレッドを起動する
        let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand<H::UserData>>();
        let worker_component = component.clone();
        let worker_codec_config = codec_config.clone();
        let poll_thread = std::thread::Builder::new()
            .name("amf-encoder-worker".into())
            .spawn(move || {
                worker(worker_component, handler, cmd_rx, worker_codec_config);
            })
            .map_err(|e| {
                Error::new_custom(
                    "Encoder::new",
                    &format!("failed to spawn worker thread: {e}"),
                )
            })?;

        Ok(Self {
            component,
            context,
            surface_format,
            width: config.width as i32,
            height: config.height as i32,
            frame_count: 0,
            framerate_num: config.framerate_num as u64,
            framerate_den: config.framerate_den as u64,
            codec_config,
            cmd_tx: Some(cmd_tx),
            poll_thread: Some(poll_thread),
        })
    }

    /// エンコーダーにプロパティを設定する
    fn set_properties(prop: &PropertyStorage, config: &EncoderConfig) -> Result<(), Error> {
        match &config.codec {
            CodecConfig::H264(h264) => {
                Self::set_h264_properties(prop, config, h264)?;
            }
            CodecConfig::Hevc(hevc) => {
                Self::set_hevc_properties(prop, config, hevc)?;
            }
            CodecConfig::Av1(av1) => {
                Self::set_av1_properties(prop, config, av1)?;
            }
        }
        Ok(())
    }

    /// エンコーダーの動的プロパティを再設定する
    ///
    /// 変更は次回 `SubmitInput` 前に AMF エンコーダーへ反映される。
    pub fn reconfigure(&mut self, params: ReconfigureParams) -> Result<(), Error> {
        let framerate = Self::resolve_reconfigure_framerate(&params)?;
        let prop = self.component.property_storage()?;

        match &self.codec_config {
            CodecConfig::H264(_) => {
                Self::set_h264_dynamic_properties(&prop, &params, framerate)?;
            }
            CodecConfig::Hevc(_) => {
                Self::set_hevc_dynamic_properties(&prop, &params, framerate)?;
            }
            CodecConfig::Av1(_) => {
                Self::set_av1_dynamic_properties(&prop, &params, framerate)?;
            }
        }

        // PTS 計算に使うフレームレートも同期して更新する
        if let Some((num, den)) = framerate {
            self.framerate_num = num as u64;
            self.framerate_den = den as u64;
        }

        Ok(())
    }

    /// 再設定時のフレームレート指定を検証する
    fn resolve_reconfigure_framerate(
        params: &ReconfigureParams,
    ) -> Result<Option<(u32, u32)>, Error> {
        match (params.framerate_num, params.framerate_den) {
            (None, None) => Ok(None),
            (Some(num), Some(den)) => {
                if num == 0 || den == 0 {
                    return Err(Error::new_custom(
                        "Encoder::reconfigure",
                        &format!(
                            "invalid framerate: {num}/{den} (must be non-zero when specified)"
                        ),
                    ));
                }
                Ok(Some((num, den)))
            }
            _ => Err(Error::new_custom(
                "Encoder::reconfigure",
                "framerate_num and framerate_den must be set together",
            )),
        }
    }

    /// H.264 の動的プロパティを設定する
    fn set_h264_dynamic_properties(
        prop: &PropertyStorage,
        params: &ReconfigureParams,
        framerate: Option<(u32, u32)>,
    ) -> Result<(), Error> {
        Self::set_codec_dynamic_properties(
            prop,
            params,
            framerate,
            sys::str::AMF_VIDEO_ENCODER_FRAMERATE,
            sys::str::AMF_VIDEO_ENCODER_TARGET_BITRATE,
            sys::str::AMF_VIDEO_ENCODER_PEAK_BITRATE,
            sys::str::AMF_VIDEO_ENCODER_QP_I,
            sys::str::AMF_VIDEO_ENCODER_QP_P,
            Some(sys::str::AMF_VIDEO_ENCODER_QP_B),
            Some(sys::str::AMF_VIDEO_ENCODER_IDR_PERIOD),
        )
    }

    /// HEVC の動的プロパティを設定する
    fn set_hevc_dynamic_properties(
        prop: &PropertyStorage,
        params: &ReconfigureParams,
        framerate: Option<(u32, u32)>,
    ) -> Result<(), Error> {
        Self::set_codec_dynamic_properties(
            prop,
            params,
            framerate,
            sys::str::AMF_VIDEO_ENCODER_HEVC_FRAMERATE,
            sys::str::AMF_VIDEO_ENCODER_HEVC_TARGET_BITRATE,
            sys::str::AMF_VIDEO_ENCODER_HEVC_PEAK_BITRATE,
            sys::str::AMF_VIDEO_ENCODER_HEVC_QP_I,
            sys::str::AMF_VIDEO_ENCODER_HEVC_QP_P,
            // AMF HEVC エンコーダーには B フレーム用の QP プロパティが存在しないため
            // qpb は設定しない (H.264 の AMF_VIDEO_ENCODER_QP_B に相当するものがない)
            None,
            None,
        )
    }

    /// AV1 の動的プロパティを設定する
    fn set_av1_dynamic_properties(
        prop: &PropertyStorage,
        params: &ReconfigureParams,
        framerate: Option<(u32, u32)>,
    ) -> Result<(), Error> {
        Self::set_codec_dynamic_properties(
            prop,
            params,
            framerate,
            sys::str::AMF_VIDEO_ENCODER_AV1_FRAMERATE,
            sys::str::AMF_VIDEO_ENCODER_AV1_TARGET_BITRATE,
            sys::str::AMF_VIDEO_ENCODER_AV1_PEAK_BITRATE,
            sys::str::AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTRA,
            sys::str::AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTER,
            Some(sys::str::AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTER_B),
            Some(sys::str::AMF_VIDEO_ENCODER_AV1_GOP_SIZE),
        )
    }

    /// codec 共通の動的プロパティを設定する
    #[allow(clippy::too_many_arguments)]
    fn set_codec_dynamic_properties(
        prop: &PropertyStorage,
        params: &ReconfigureParams,
        framerate: Option<(u32, u32)>,
        framerate_name: &'static str,
        target_bitrate_name: &'static str,
        peak_bitrate_name: &'static str,
        qp_i_name: &'static str,
        qp_p_name: &'static str,
        qp_b_name: Option<&'static str>,
        gop_pic_size_name: Option<&'static str>,
    ) -> Result<(), Error> {
        if let Some((num, den)) = framerate {
            prop.set_property_rate(framerate_name, num, den)?;
        }
        if let Some(target_kbps) = params.target_kbps {
            prop.set_property_int64(target_bitrate_name, target_kbps as amf_int64 * 1000)?;
        }
        if let Some(max_kbps) = params.max_kbps {
            prop.set_property_int64(peak_bitrate_name, max_kbps as amf_int64 * 1000)?;
        }
        if let Some(qpi) = params.qpi {
            prop.set_property_int64(qp_i_name, qpi as amf_int64)?;
        }
        if let Some(qpp) = params.qpp {
            prop.set_property_int64(qp_p_name, qpp as amf_int64)?;
        }
        if let (Some(qpb_name), Some(qpb)) = (qp_b_name, params.qpb) {
            prop.set_property_int64(qpb_name, qpb as amf_int64)?;
        }
        if let (Some(gop_name), Some(gop)) = (gop_pic_size_name, params.gop_pic_size) {
            prop.set_property_int64(gop_name, gop as amf_int64)?;
        }
        Ok(())
    }

    /// H.264 固有のプロパティを設定する
    fn set_h264_properties(
        prop: &PropertyStorage,
        config: &EncoderConfig,
        h264: &H264EncoderConfig,
    ) -> Result<(), Error> {
        // Usage: トランスコーディング
        prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_USAGE, 0)?;

        // Profile
        if let Some(profile) = h264.profile {
            let profile_val: amf_int64 = match profile {
                H264Profile::Baseline => 66,
                H264Profile::Main => 77,
                H264Profile::High => 100,
                H264Profile::ConstrainedBaseline => 256,
                H264Profile::ConstrainedHigh => 257,
            };
            prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_PROFILE, profile_val)?;
        }

        // Frame size
        prop.set_property_size(
            sys::str::AMF_VIDEO_ENCODER_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
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
        prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD, rc_method)?;

        let params = ReconfigureParams {
            target_kbps: config.target_kbps,
            max_kbps: config.max_kbps,
            qpi: config.qpi,
            qpp: config.qpp,
            qpb: config.qpb,
            gop_pic_size: config.gop_pic_size,
            ..ReconfigureParams::default()
        };
        Self::set_h264_dynamic_properties(
            prop,
            &params,
            Some((config.framerate_num, config.framerate_den)),
        )?;

        Ok(())
    }

    /// HEVC 固有のプロパティを設定する
    fn set_hevc_properties(
        prop: &PropertyStorage,
        config: &EncoderConfig,
        hevc: &HevcEncoderConfig,
    ) -> Result<(), Error> {
        prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_HEVC_USAGE, 0)?;

        if let Some(profile) = hevc.profile {
            let profile_val: amf_int64 = match profile {
                HevcProfile::Main => 1,
                HevcProfile::Main10 => 2,
            };
            prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_HEVC_PROFILE, profile_val)?;
        }

        prop.set_property_size(
            sys::str::AMF_VIDEO_ENCODER_HEVC_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
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
        prop.set_property_int64(
            sys::str::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD,
            rc_method,
        )?;

        let params = ReconfigureParams {
            target_kbps: config.target_kbps,
            max_kbps: config.max_kbps,
            qpi: config.qpi,
            qpp: config.qpp,
            ..ReconfigureParams::default()
        };
        Self::set_hevc_dynamic_properties(
            prop,
            &params,
            Some((config.framerate_num, config.framerate_den)),
        )?;

        if let Some(gop) = config.gop_pic_size {
            prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_HEVC_GOP_SIZE, gop as amf_int64)?;
        }

        Ok(())
    }

    /// AV1 固有のプロパティを設定する
    fn set_av1_properties(
        prop: &PropertyStorage,
        config: &EncoderConfig,
        _av1: &Av1EncoderConfig,
    ) -> Result<(), Error> {
        prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_AV1_USAGE, 0)?;

        prop.set_property_size(
            sys::str::AMF_VIDEO_ENCODER_AV1_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
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
        prop.set_property_int64(
            sys::str::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD,
            rc_method,
        )?;

        let params = ReconfigureParams {
            target_kbps: config.target_kbps,
            max_kbps: config.max_kbps,
            qpi: config.qpi,
            qpp: config.qpp,
            // AV1 の初期設定では従来どおり qpb は設定しない
            qpb: None,
            gop_pic_size: config.gop_pic_size,
            ..ReconfigureParams::default()
        };
        Self::set_av1_dynamic_properties(
            prop,
            &params,
            Some((config.framerate_num, config.framerate_den)),
        )?;

        Ok(())
    }

    /// エンコード用の Surface を確保する
    ///
    /// 確保された Surface は呼び出し元がフレームデータを書き込んでから
    /// [`encode()`] に渡す。
    pub fn alloc_surface(&self) -> Result<Surface, Error> {
        self.context.alloc_surface(
            AMF_MEMORY_TYPE::AMF_MEMORY_HOST,
            self.surface_format,
            self.width,
            self.height,
        )
    }

    /// フレームをエンコードする
    ///
    /// `user_data` はエンコード完了時にハンドラーへ渡される。
    pub fn encode(
        &mut self,
        surface: Surface,
        options: &EncodeOptions,
        user_data: H::UserData,
    ) -> Result<(), Error> {
        // PTS を設定する
        let pts = (self.frame_count * self.framerate_den * AMF_SECOND as u64 / self.framerate_num)
            as amf_pts;
        surface.set_pts(pts);

        // フレームタイプの強制
        if options.frame_type & frame_type::IDR != 0 {
            self.force_picture_type(&surface)?;
        }

        log::debug!("Encoder::encode: SubmitInput");
        // SubmitInput (INPUT_FULL の場合はリトライする)
        let max_retries = 100;
        let mut submitted = false;
        for retry in 0..max_retries {
            let result = unsafe {
                self.component
                    .submit_input(surface.as_ptr() as *mut AMFData)
            };

            if result == AMF_RESULT::AMF_OK {
                submitted = true;
                break;
            }
            if result == AMF_RESULT::AMF_INPUT_FULL || result == AMF_RESULT::AMF_REPEAT {
                if retry > 0 && retry % 10 == 0 {
                    log::debug!("Encoder::SubmitInput: retry={retry}, result={result:?}");
                }
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            // その他のエラー
            return Error::check(result, "AMFComponent::SubmitInput");
        }
        if !submitted {
            return Err(Error::new_custom(
                "Encoder::encode",
                "SubmitInput retry limit exceeded",
            ));
        }

        self.frame_count += 1;

        // ワーカースレッドに Submit コマンドを送信する
        self.cmd_tx
            .as_ref()
            .unwrap()
            .send(WorkerCommand::Submit(user_data))
            .map_err(|_| Error::new_custom("Encoder::encode", "worker thread terminated"))?;

        Ok(())
    }

    /// IDR フレームを強制する
    fn force_picture_type(&self, surface: &Surface) -> Result<(), Error> {
        let prop = surface.property_storage()?;
        match &self.codec_config {
            CodecConfig::H264(_) => {
                prop.set_property_int64(
                    sys::str::AMF_VIDEO_ENCODER_FORCE_PICTURE_TYPE,
                    sys::AMF_VIDEO_ENCODER_PICTURE_TYPE_ENUM_AMF_VIDEO_ENCODER_PICTURE_TYPE_IDR
                        .into(),
                )?;
                prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_INSERT_SPS, 1)?;
                prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_INSERT_PPS, 1)?;
            }
            CodecConfig::Hevc(_) => {
                prop.set_property_int64(
                    sys::str::AMF_VIDEO_ENCODER_HEVC_FORCE_PICTURE_TYPE,
                    sys::AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_IDR.into(),
                )?;
                prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_HEVC_INSERT_HEADER, 1)?;
            }
            CodecConfig::Av1(_) => {
                prop.set_property_int64(
                    sys::str::AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE,
                    sys::AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE_KEY.into(),
                )?;
                prop.set_property_int64(
                    sys::str::AMF_VIDEO_ENCODER_AV1_FORCE_INSERT_SEQUENCE_HEADER,
                    1,
                )?;
            }
        }
        Ok(())
    }

    /// エンコーダーをフラッシュして残りのフレームを処理する
    pub fn finish(&mut self) -> Result<(), Error> {
        // Drain を呼び出して残りのフレームを出力させる
        let result = self.component.drain();
        // AMF_INPUT_FULL は Drain 中に返ることがあるが無視する
        if result != AMF_RESULT::AMF_OK && result != AMF_RESULT::AMF_INPUT_FULL {
            Error::check(result, "AMFComponent::Drain")?;
        }

        // Finish コマンドでワーカースレッドに残りの出力を処理させる
        let (tx, rx) = mpsc::sync_channel(1);
        self.cmd_tx
            .as_ref()
            .unwrap()
            .send(WorkerCommand::Finish(tx))
            .map_err(|_| Error::new_custom("Encoder::finish", "worker thread terminated"))?;

        // 全 pending が処理されるのを待つ
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| Error::new_custom("Encoder::finish", "Finish wait timed out"))?;

        Ok(())
    }
}

// 安全性:
// Drop 内でのみ component/context を解放する。
// ワーカースレッドは Drop より先に停止させる。
impl<H: EncodeHandler> Drop for Encoder<H> {
    fn drop(&mut self) {
        // ワーカースレッドを停止する (cmd_tx を drop → チャネル切断 → ワーカー終了)
        self.cmd_tx = None;

        if let Some(handle) = self.poll_thread.take() {
            let _ = handle.join();
        }

        let _ = self.component.terminate();
        let _ = self.context.terminate();
    }
}

// ---------------------------------------------------------------------------
// ワーカースレッド
// ---------------------------------------------------------------------------

/// ワーカースレッドのエントリポイント
///
/// メインスレッドから `Submit(T)` コマンドを受け取り、AMFComponent::QueryOutput を
/// ポーリングしてエンコード済みフレームを取得し、ハンドラーを呼び出す。
fn worker<H: EncodeHandler>(
    component: Component,
    mut handler: H,
    cmd_rx: mpsc::Receiver<WorkerCommand<H::UserData>>,
    codec_config: CodecConfig,
) {
    let mut pending: VecDeque<H::UserData> = VecDeque::new();
    let mut output_buffer: VecDeque<Result<(Buffer, PictureType), crate::Error>> = VecDeque::new();
    let mut finish: Option<mpsc::SyncSender<()>> = None;

    loop {
        // Finish がやってきていて、全てのデータが排出されたら終了する
        if finish.is_some() && pending.is_empty() {
            break;
        }

        let cmd = if pending.is_empty() {
            match cmd_rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            }
        } else {
            match cmd_rx.recv_timeout(Duration::from_millis(1)) {
                Ok(cmd) => cmd,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    drain_output(
                        &mut output_buffer,
                        &mut pending,
                        &mut handler,
                        &component,
                        &codec_config,
                    );
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };

        match cmd {
            WorkerCommand::Submit(t) => {
                pending.push_back(t);
            }
            WorkerCommand::Finish(tx) => {
                finish = Some(tx);
            }
        }
    }

    if let Some(tx) = finish {
        let _ = tx.send(());
    }
}

/// QueryOutput からの出力をバッファに格納し、pending とマッチングしてハンドラーを呼び出す
fn drain_output<H: EncodeHandler>(
    output_buffer: &mut VecDeque<Result<(Buffer, PictureType), crate::Error>>,
    pending: &mut VecDeque<H::UserData>,
    handler: &mut H,
    component: &Component,
    codec_config: &CodecConfig,
) {
    // 出力をバッファに格納する
    loop {
        let mut data: *mut AMFData = ptr::null_mut();
        let result = unsafe { component.query_output(&mut data) };
        log::debug!("worker: QueryOutput result={result:?}");
        if result == AMF_RESULT::AMF_REPEAT || result == AMF_RESULT::AMF_EOF {
            break;
        }
        if result != AMF_RESULT::AMF_OK || data.is_null() {
            break;
        }
        output_buffer.push_back(extract_encoded_output(data, codec_config));
    }

    // バッファされた出力と pending をマッチングする
    while !output_buffer.is_empty() && !pending.is_empty() {
        let output = output_buffer.pop_front().unwrap();
        let user_data = pending.pop_front().unwrap();
        handler.on_encoded(
            output
                .map(|(buffer, picture_type)| EncodedFrame {
                    buffer,
                    picture_type,
                    user_data,
                })
                .map_err(Into::into),
        );
    }
}

/// AMFData から Buffer とピクチャタイプを抽出する
fn extract_encoded_output(
    data: *mut AMFData,
    codec_config: &CodecConfig,
) -> Result<(Buffer, PictureType), Error> {
    let buffer = data as *mut AMFBuffer;
    if buffer.is_null() {
        return Err(Error::new_custom(
            "extract_encoded_output",
            "buffer is null",
        ));
    }
    let buffer = unsafe { Buffer::from_raw(buffer) }?;
    let buf_size = buffer.get_size();
    if buf_size == 0 {
        return Err(Error::new_custom(
            "extract_encoded_output",
            "buffer size is 0",
        ));
    }
    let buf_native = buffer.get_native() as *const u8;
    if buf_native.is_null() {
        return Err(Error::new_custom(
            "extract_encoded_output",
            "buffer native is null",
        ));
    }
    let picture_type = get_output_picture_type(&buffer, codec_config)?;
    Ok((buffer, picture_type))
}

/// 出力バッファからピクチャタイプを取得する
fn get_output_picture_type(
    buffer: &Buffer,
    codec_config: &CodecConfig,
) -> Result<PictureType, Error> {
    let prop_name = match codec_config {
        CodecConfig::H264(_) => sys::str::AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE,
        CodecConfig::Hevc(_) => sys::str::AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE,
        CodecConfig::Av1(_) => sys::str::AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE,
    };
    let name_w = sys::to_wstring(prop_name);
    let mut var = AMFVariantStruct::empty();
    let result = unsafe { buffer.get_property(name_w.as_ptr(), &mut var) };
    if result != AMF_RESULT::AMF_OK {
        return Err(Error::new_custom(
            "get_output_picture_type",
            &format!("GetProperty failed: {result:?}"),
        ));
    }

    let type_val = unsafe { var.__bindgen_anon_1.int64Value };
    let picture_type = match codec_config {
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
    };
    Ok(picture_type)
}
