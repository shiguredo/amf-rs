//! AMF ハードウェアエンコーダー
//!
//! AMD GPU を使ったハードウェアビデオエンコードを提供する。
//! H.264/AVC、H.265/HEVC、AV1 コーデックに対応する。

use crate::AmfLibrary;
use crate::amf::{Buffer, Component, Context, PropertyStorage, Surface};
use crate::error::{AmfError, Error, InternalError, InvalidArgumentError, InvalidStateError};
use crate::sys::{self, AMF_RESULT, amf_int32, amf_int64};
use crate::types::{Av1Profile, FrameFormat, H264Profile, HevcProfile, MemoryType, SurfaceFormat};
use crate::worker::{Worker, WorkerConfig};

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

impl FrameFormat {
    /// このフレームフォーマットに対応する出力ビット深度を返す
    ///
    /// AMF が扱えない 12bit / 16bit のフォーマットでは `None` を返し、
    /// AMF の既定 (8bit) に任せる。
    fn output_color_bit_depth(self) -> Option<amf_int64> {
        // AMF_COLOR_BIT_DEPTH_ENUM には 8bit と 10bit しかない
        match self {
            FrameFormat::Nv12
            | FrameFormat::Yv12
            | FrameFormat::I420
            | FrameFormat::Bgra
            | FrameFormat::Argb
            | FrameFormat::Rgba
            | FrameFormat::Yuy2
            | FrameFormat::Uyvy
            | FrameFormat::Ayuv => {
                Some(sys::AMF_COLOR_BIT_DEPTH_ENUM_AMF_COLOR_BIT_DEPTH_8 as amf_int64)
            }
            FrameFormat::P010 | FrameFormat::Y210 | FrameFormat::Y410 => {
                Some(sys::AMF_COLOR_BIT_DEPTH_ENUM_AMF_COLOR_BIT_DEPTH_10 as amf_int64)
            }
            FrameFormat::P012 | FrameFormat::P016 | FrameFormat::Y416 => None,
        }
    }

    /// 10bit のフォーマットか
    fn is_10bit(self) -> bool {
        matches!(
            self,
            FrameFormat::P010 | FrameFormat::Y210 | FrameFormat::Y410
        )
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

impl RateControlMode {
    /// H.264 エンコーダーのレート制御メソッドへ変換する
    fn to_h264(self) -> sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM {
        match self {
            RateControlMode::Cqp => {
                sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_CONSTANT_QP
            }
            RateControlMode::Cbr => {
                sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_CBR
            }
            RateControlMode::Vbr => {
                sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR
            }
            RateControlMode::LatencyConstrainedVbr => {
                sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR
            }
            RateControlMode::QualityVbr => {
                sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_QUALITY_VBR
            }
            RateControlMode::HighQualityVbr => {
                sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR
            }
            RateControlMode::HighQualityCbr => {
                sys::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR
            }
        }
    }

    /// HEVC エンコーダーのレート制御メソッドへ変換する
    fn to_hevc(self) -> sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM {
        match self {
            RateControlMode::Cqp => {
                sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_CONSTANT_QP
            }
            RateControlMode::Cbr => {
                sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_CBR
            }
            RateControlMode::Vbr => {
                sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR
            }
            RateControlMode::LatencyConstrainedVbr => {
                sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR
            }
            RateControlMode::QualityVbr => {
                sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_QUALITY_VBR
            }
            RateControlMode::HighQualityVbr => {
                sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR
            }
            RateControlMode::HighQualityCbr => {
                sys::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR
            }
        }
    }

    /// AV1 エンコーダーのレート制御メソッドへ変換する
    fn to_av1(self) -> sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM {
        match self {
            RateControlMode::Cqp => {
                sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_CONSTANT_QP
            }
            RateControlMode::Cbr => {
                sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_CBR
            }
            RateControlMode::Vbr => {
                sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR
            }
            RateControlMode::LatencyConstrainedVbr => {
                sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR
            }
            RateControlMode::QualityVbr => {
                sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_QUALITY_VBR
            }
            RateControlMode::HighQualityVbr => {
                sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR
            }
            RateControlMode::HighQualityCbr => {
                sys::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR
            }
        }
    }
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

/// H.264 エンコーダー固有設定
#[derive(Debug, Clone)]
pub struct H264EncoderConfig {
    /// プロファイル
    pub profile: Option<H264Profile>,
}

/// HEVC エンコーダー固有設定
#[derive(Debug, Clone)]
pub struct HevcEncoderConfig {
    /// プロファイル
    ///
    /// 10bit の入力フォーマットでは `Some(HevcProfile::Main10)` を指定する必要があり、
    /// `Some(HevcProfile::Main)` や `None` を指定した場合は [`Encoder::new`] が
    /// エラーを返す。
    pub profile: Option<HevcProfile>,
}

/// AV1 エンコーダー固有設定
#[derive(Debug, Clone)]
pub struct Av1EncoderConfig {
    /// プロファイル
    pub profile: Option<Av1Profile>,
}

/// コーデックとコーデック固有設定
///
/// [`EncoderConfig::codec`] に指定する。コーデック固有設定のプロファイルが
/// `None` の場合は AMF の既定のプロファイルを使う。
#[derive(Debug, Clone)]
pub enum CodecConfig {
    /// H.264/AVC
    H264(H264EncoderConfig),
    /// H.265/HEVC
    Hevc(HevcEncoderConfig),
    /// AV1
    Av1(Av1EncoderConfig),
}

/// エンコーダー設定
///
/// `width` と `height` は 0 でない偶数で、`i32::MAX` 以下を指定する。
/// `framerate_num` と `framerate_den` は 0 でない値で、フレームレートを表す。
/// `Option` のフィールドは `None` のとき AMF の既定値を使う。
/// QP の有効範囲は H.264 と HEVC が 0-51、AV1 が 1-255 である。
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// 使用するコーデック
    pub codec: CodecConfig,
    /// エンコードするフレームの幅 (ピクセル単位)
    pub width: u32,
    /// エンコードするフレームの高さ (ピクセル単位)
    pub height: u32,
    /// 入力フレームのフォーマット
    ///
    /// H.264 は 8bit のフォーマットだけを使える。HEVC と AV1 は 8bit と 10bit の
    /// フォーマットを使えるが、利用できるかは GPU と AMF の実装に依存する。
    /// 12bit と 16bit のフォーマット (P012 / P016 / Y416) は初期化に失敗する。
    pub frame_format: FrameFormat,
    /// フレームレートの分子
    pub framerate_num: u32,
    /// フレームレートの分母
    pub framerate_den: u32,
    /// レート制御モード
    ///
    /// `target_kbps` と `max_kbps` の使われ方はこのモードによって変わる。
    pub rate_control_mode: RateControlMode,
    /// 目標ビットレート (kbps 単位)
    ///
    /// CBR と VBR 系のレート制御で使う。
    pub target_kbps: Option<u32>,
    /// ピークビットレート (kbps 単位)
    ///
    /// VBR 系のレート制御で使う。
    pub max_kbps: Option<u32>,
    /// I フレームの QP
    pub qpi: Option<u16>,
    /// P フレームの QP
    pub qpp: Option<u16>,
    /// B フレームの QP
    ///
    /// HEVC では指定できない ([`Encoder::new`] がエラーを返す)。
    pub qpb: Option<u16>,
    /// GOP サイズ (フレーム単位)
    ///
    /// H.264 では IDR フレームの間隔、HEVC と AV1 ではキーフレームの間隔になる。
    pub gop_pic_size: Option<u16>,
    /// GOP 内の連続する B フレーム数
    ///
    /// `0` は B フレームを使わない。`None` のとき AMF の既定値 (`0`) を使う。
    /// AMF の仕様上の値域は H.264 が `0..=3`、AV1 が `0..=127` である。
    /// HEVC では指定できない ([`Encoder::new`] がエラーを返す)。
    pub b_pic_pattern: Option<u8>,
}

impl EncoderConfig {
    /// エンコーダー設定を作成する
    ///
    /// 必須の項目を指定し、`Option` のフィールドは `None` で初期化する。
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
            b_pic_pattern: None,
        }
    }
}

/// エンコードオプション（フレームごと）
#[derive(Debug, Clone, Copy, Default)]
pub struct EncodeOptions {
    /// 強制するフレームタイプ
    ///
    /// [`frame_type`] の定数をビット OR で指定する。`frame_type::UNKNOWN` の場合は
    /// 強制しない。指定した値の扱いは [`frame_type`] モジュールを参照すること。
    pub frame_type: u16,
}

/// エンコーダー再設定パラメータ
///
/// `None` の項目は変更しない。各フィールドの意味は [`EncoderConfig`] の対応する
/// フィールドと同じである。`framerate_num` と `framerate_den` は 0 でない値で、
/// 同時に指定する。HEVC では `qpb` と `gop_pic_size` を指定すると
/// [`Encoder::reconfigure`] がエラーを返す。
#[derive(Debug, Clone, Default)]
pub struct ReconfigureParams {
    /// フレームレートの分子
    pub framerate_num: Option<u32>,
    /// フレームレートの分母
    pub framerate_den: Option<u32>,
    /// 目標ビットレート (kbps 単位)
    pub target_kbps: Option<u32>,
    /// ピークビットレート (kbps 単位)
    pub max_kbps: Option<u32>,
    /// I フレームの QP
    pub qpi: Option<u16>,
    /// P フレームの QP
    pub qpp: Option<u16>,
    /// B フレームの QP
    pub qpb: Option<u16>,
    /// GOP サイズ (フレーム単位)
    ///
    /// H.264 では IDR フレームの間隔、AV1 ではキーフレームの間隔になる。
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

    /// エンコード時に渡したユーザーデータ
    pub fn user_data(&self) -> &T {
        &self.user_data
    }

    /// ビットストリームバッファとユーザーデータの所有権を取得する
    pub fn into_parts(self) -> (Buffer, T) {
        (self.buffer, self.user_data)
    }
}

// ---------------------------------------------------------------------------
// frame_type 定数
// ---------------------------------------------------------------------------
/// 強制するフレームタイプを表すビットフラグ
///
/// [`EncodeOptions::frame_type`] に指定する。AMF のピクチャタイプ enum とは
/// 独立したビットフラグである。`IDR` / `I` / `P` が指定された場合は `IDR`、`I`、
/// `P` の順に優先して 1 つを AMF の強制ピクチャタイプへ変換する。
///
/// `B` と `REF`、未定義のビット、利用するコーデックが対応しないフラグは無視する。
/// B フレームを生成するには [`EncoderConfig::b_pic_pattern`] を指定する。
/// 先頭フレームと GOP の境界では、エンコーダーが IDR を優先するため `I` と `P`
/// の指定が反映されない場合がある。
pub mod frame_type {
    /// フレームタイプを強制しない
    pub const UNKNOWN: u16 = 0;
    /// I フレーム
    pub const I: u16 = 0x0001;
    /// P フレーム
    pub const P: u16 = 0x0002;
    /// B フレーム
    pub const B: u16 = 0x0004;
    /// IDR フレーム
    ///
    /// キーフレームを強制し、H.264 は SPS / PPS、HEVC は VPS / SPS / PPS、
    /// AV1 はシーケンスヘッダーを出力に含める。
    pub const IDR: u16 = 0x0020;
    /// 参照フレーム
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
    /// クロージャからハンドラーを作成する
    ///
    /// `f` は [`EncodeHandler::on_encoded`] と同じ引数で、エンコード完了時に
    /// ワーカースレッドから呼ばれる。
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

/// コーデックごとに異なる動的プロパティの名前
struct DynamicPropertyNames {
    /// フレームレート
    framerate: &'static str,
    /// 目標ビットレート
    target_bitrate: &'static str,
    /// ピークビットレート
    peak_bitrate: &'static str,
    /// I フレームの QP
    qp_i: &'static str,
    /// P フレームの QP
    qp_p: &'static str,
    /// B フレームの QP (対応するプロパティがないコーデックでは `None`)
    qp_b: Option<&'static str>,
    /// GOP サイズ (対応するプロパティがないコーデックでは `None`)
    gop_pic_size: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// エンコーダー実装
// ---------------------------------------------------------------------------

/// AMF ハードウェアエンコーダー
///
/// エンコード結果は、[`Encoder::new`] に渡したハンドラーへワーカースレッドから通知される。
///
/// [`Encoder::encode`] / [`Encoder::finish`] がエラーを返した場合、その後は
/// `Drop` 以外の操作を保証しない。`Drop` は安全に呼び出せる。
///
/// # Drop
///
/// [`Encoder::finish`] を呼ばずに drop した場合、drop はワーカーが停止するまで
/// 待つ。停止処理の途中で、すでに [`Encoder::encode`] が成功したフレームの出力が
/// 通知されることがある。drop は、すべてのフレームの出力が通知されることを
/// 保証しない。
pub struct Encoder<H: EncodeHandler> {
    component: Component,
    context: Context,
    surface_format: SurfaceFormat,
    width: i32,
    height: i32,
    codec_config: CodecConfig,
    /// ワーカースレッドの制御
    worker: Worker<H::UserData>,
    /// AMF が入力を受け付けており、終了時に Drain が必要か
    drain_required: bool,
}

impl<H: EncodeHandler> Encoder<H> {
    /// エンコーダーを作成して初期化する
    ///
    /// `handler` はエンコード完了時にワーカースレッドから呼び出される。
    ///
    /// # 入力フォーマットとプロファイル
    ///
    /// 入力フォーマットのビット深度と利用可否は [`EncoderConfig::frame_format`]、
    /// HEVC のプロファイルの制約は [`HevcEncoderConfig::profile`] を参照すること。
    ///
    /// # Errors
    ///
    /// `config` の値が不正な場合、または AMF のエンコーダーを初期化できない場合に
    /// エラーを返す。GPU と AMF の実装が指定した設定に対応していない場合も含む。
    pub fn new(context: Context, config: EncoderConfig, mut handler: H) -> Result<Self, Error> {
        // 入力パラメータのバリデーション
        if config.width == 0 || config.height == 0 {
            return Err(InvalidArgumentError::new(format!(
                "invalid resolution: {}x{} (must be non-zero)",
                config.width, config.height
            ))
            .into());
        }
        if config.width > i32::MAX as u32 || config.height > i32::MAX as u32 {
            return Err(InvalidArgumentError::new(format!(
                "invalid resolution: {}x{} (exceeds i32::MAX)",
                config.width, config.height
            ))
            .into());
        }
        if !config.width.is_multiple_of(2) || !config.height.is_multiple_of(2) {
            return Err(InvalidArgumentError::new(format!(
                "invalid resolution: {}x{} (must be even)",
                config.width, config.height
            ))
            .into());
        }
        if config.framerate_num == 0 || config.framerate_den == 0 {
            return Err(InvalidArgumentError::new(format!(
                "invalid framerate: {}/{} (must be non-zero)",
                config.framerate_num, config.framerate_den
            ))
            .into());
        }

        if matches!(&config.codec, CodecConfig::Hevc(_)) && config.qpb.is_some() {
            return Err(InvalidArgumentError::new("qpb is not supported for HEVC").into());
        }
        // HEVC の Main プロファイルと 10bit 入力の組み合わせは、AMF が Main に
        // 適合しない 10bit ストリームを生成するため、事前に拒否する。
        Self::validate_hevc_bit_depth(&config)?;

        let lib = AmfLibrary::instance();

        // コーデック固有のコンポーネント ID を選択する
        let component_id = match &config.codec {
            CodecConfig::H264(_) => sys::str::AMFVideoEncoderVCE_AVC,
            CodecConfig::Hevc(_) => sys::str::AMFVideoEncoder_HEVC,
            CodecConfig::Av1(_) => sys::str::AMFVideoEncoder_AV1,
        };

        // コンポーネントを作成する
        let component = lib.create_component(&context, component_id)?;

        // B フレームを設定できるかどうか確認する。
        // AMF は、B フレーム非対応の GPU であってもプロパティの設定を受理して
        // 成功してしまうことを確認している（当然 B フレームは生成されない）。
        // そのため、B フレームの設定を確認し、非対応と確認できたものに関しては
        // 拒否するチェックを入れる。
        if let Some(pattern) = config.b_pic_pattern {
            let cap_name = match &config.codec {
                CodecConfig::H264(_) => sys::str::AMF_VIDEO_ENCODER_CAP_BFRAMES,
                CodecConfig::Av1(_) => sys::str::AMF_VIDEO_ENCODER_AV1_CAP_BFRAMES,
                CodecConfig::Hevc(_) => {
                    return Err(InvalidArgumentError::new(
                        "b_pic_pattern is not supported for HEVC",
                    )
                    .into());
                }
            };
            if pattern > 0 {
                let capability = component
                    .get_caps()
                    .ok()
                    .and_then(|caps| caps.get_property(cap_name))
                    .and_then(|value| value.as_bool());
                if capability == Some(false) {
                    return Err(InvalidArgumentError::new(
                        "this GPU and AMF implementation do not support B frames",
                    )
                    .into());
                }
            }
        }

        let surface_format = SurfaceFormat::from(config.frame_format);

        // プロパティを設定する
        let prop = component.property_storage()?;
        Self::set_properties(&prop, &config)?;

        // エンコーダーを初期化する
        let result = component.init(
            surface_format,
            config.width as amf_int32,
            config.height as amf_int32,
        );
        AmfError::check(result, "AMFComponent::Init")?;

        let codec_config = config.codec;

        // ワーカースレッドを起動する
        let worker_codec_config = codec_config.clone();
        let worker = Worker::spawn(
            component.clone(),
            WorkerConfig {
                role: "encoder",
                deliver: Box::new(move |result| {
                    handler.on_encoded(
                        result
                            .and_then(|(data, user_data)| {
                                let buffer = data.to_buffer()?;
                                let (buffer, picture_type) =
                                    validate_encoded_output(buffer, &worker_codec_config)?;
                                Ok(EncodedFrame {
                                    buffer,
                                    picture_type,
                                    user_data,
                                })
                            })
                            .map_err(Into::into),
                    );
                }),
            },
        )?;

        Ok(Self {
            component,
            context,
            surface_format,
            width: config.width as i32,
            height: config.height as i32,
            codec_config,
            worker,
            drain_required: false,
        })
    }

    /// HEVC の入力ビット深度とプロファイルの組み合わせを検証する
    ///
    /// AMF は HEVC の Main プロファイルと 10bit 入力の組み合わせをエラーにせず、
    /// Main プロファイルに適合しない 10bit ストリームを生成する。この組み合わせだけは
    /// ライブラリ側で拒否する。
    fn validate_hevc_bit_depth(config: &EncoderConfig) -> Result<(), Error> {
        let CodecConfig::Hevc(hevc) = &config.codec else {
            return Ok(());
        };
        if !config.frame_format.is_10bit() {
            return Ok(());
        }

        let profile = match hevc.profile {
            Some(HevcProfile::Main10) => return Ok(()),
            Some(HevcProfile::Main) => "HevcProfile::Main",
            None => "the default HEVC profile (Main)",
        };
        Err(InvalidArgumentError::new(format!(
            "HEVC 10bit input requires HevcProfile::Main10: {profile} with {:?} (set HevcProfile::Main10)",
            config.frame_format
        ))
        .into())
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
    ///
    /// # Errors
    ///
    /// `finish` 済みの場合、`params` の値が不正な場合、または AMF の操作に失敗
    /// した場合にエラーを返す。利用するコーデックが指定した項目に対応していない
    /// 場合も含む。
    pub fn reconfigure(&mut self, params: ReconfigureParams) -> Result<(), Error> {
        if !self.worker.is_running() {
            return Err(InvalidStateError::new("encoder is already finished").into());
        }

        // HEVC には B フレーム用の QP プロパティがない。また AMF の HEVC では
        // GOP サイズが初期化前の静的プロパティであり、H.264 の IDRPeriod や
        // AV1 の GOPSize と異なり初期化後には変更できない。再設定では指定できない。
        if let CodecConfig::Hevc(_) = &self.codec_config {
            if params.qpb.is_some() {
                return Err(InvalidArgumentError::new("qpb is not supported for HEVC").into());
            }
            if params.gop_pic_size.is_some() {
                return Err(
                    InvalidArgumentError::new("gop_pic_size cannot be changed for HEVC").into(),
                );
            }
        }

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
                    return Err(InvalidArgumentError::new(format!(
                        "invalid framerate: {num}/{den} (must be non-zero when specified)"
                    ))
                    .into());
                }
                Ok(Some((num, den)))
            }
            _ => Err(InvalidArgumentError::new(
                "framerate_num and framerate_den must be set together",
            )
            .into()),
        }
    }

    /// H.264 の動的プロパティを設定する
    fn set_h264_dynamic_properties(
        prop: &PropertyStorage,
        params: &ReconfigureParams,
        framerate: Option<(u32, u32)>,
    ) -> Result<(), Error> {
        let names = DynamicPropertyNames {
            framerate: sys::str::AMF_VIDEO_ENCODER_FRAMERATE,
            target_bitrate: sys::str::AMF_VIDEO_ENCODER_TARGET_BITRATE,
            peak_bitrate: sys::str::AMF_VIDEO_ENCODER_PEAK_BITRATE,
            qp_i: sys::str::AMF_VIDEO_ENCODER_QP_I,
            qp_p: sys::str::AMF_VIDEO_ENCODER_QP_P,
            qp_b: Some(sys::str::AMF_VIDEO_ENCODER_QP_B),
            gop_pic_size: Some(sys::str::AMF_VIDEO_ENCODER_IDR_PERIOD),
        };
        Self::set_codec_dynamic_properties(prop, params, framerate, &names)
    }

    /// HEVC の動的プロパティを設定する
    fn set_hevc_dynamic_properties(
        prop: &PropertyStorage,
        params: &ReconfigureParams,
        framerate: Option<(u32, u32)>,
    ) -> Result<(), Error> {
        let names = DynamicPropertyNames {
            framerate: sys::str::AMF_VIDEO_ENCODER_HEVC_FRAMERATE,
            target_bitrate: sys::str::AMF_VIDEO_ENCODER_HEVC_TARGET_BITRATE,
            peak_bitrate: sys::str::AMF_VIDEO_ENCODER_HEVC_PEAK_BITRATE,
            qp_i: sys::str::AMF_VIDEO_ENCODER_HEVC_QP_I,
            qp_p: sys::str::AMF_VIDEO_ENCODER_HEVC_QP_P,
            // AMF HEVC エンコーダーには B フレーム用の QP プロパティが存在しないため
            // qpb は設定しない (H.264 の AMF_VIDEO_ENCODER_QP_B に相当するものがない)
            qp_b: None,
            gop_pic_size: None,
        };
        Self::set_codec_dynamic_properties(prop, params, framerate, &names)
    }

    /// AV1 の動的プロパティを設定する
    fn set_av1_dynamic_properties(
        prop: &PropertyStorage,
        params: &ReconfigureParams,
        framerate: Option<(u32, u32)>,
    ) -> Result<(), Error> {
        let names = DynamicPropertyNames {
            framerate: sys::str::AMF_VIDEO_ENCODER_AV1_FRAMERATE,
            target_bitrate: sys::str::AMF_VIDEO_ENCODER_AV1_TARGET_BITRATE,
            peak_bitrate: sys::str::AMF_VIDEO_ENCODER_AV1_PEAK_BITRATE,
            qp_i: sys::str::AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTRA,
            qp_p: sys::str::AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTER,
            qp_b: Some(sys::str::AMF_VIDEO_ENCODER_AV1_Q_INDEX_INTER_B),
            gop_pic_size: Some(sys::str::AMF_VIDEO_ENCODER_AV1_GOP_SIZE),
        };
        Self::set_codec_dynamic_properties(prop, params, framerate, &names)
    }

    /// codec 共通の動的プロパティを設定する
    fn set_codec_dynamic_properties(
        prop: &PropertyStorage,
        params: &ReconfigureParams,
        framerate: Option<(u32, u32)>,
        names: &DynamicPropertyNames,
    ) -> Result<(), Error> {
        if let Some((num, den)) = framerate {
            prop.set_property_rate(names.framerate, num, den)?;
        }
        if let Some(target_kbps) = params.target_kbps {
            prop.set_property_int64(names.target_bitrate, target_kbps as amf_int64 * 1000)?;
        }
        if let Some(max_kbps) = params.max_kbps {
            prop.set_property_int64(names.peak_bitrate, max_kbps as amf_int64 * 1000)?;
        }
        if let Some(qpi) = params.qpi {
            prop.set_property_int64(names.qp_i, qpi as amf_int64)?;
        }
        if let Some(qpp) = params.qpp {
            prop.set_property_int64(names.qp_p, qpp as amf_int64)?;
        }
        if let (Some(name), Some(qpb)) = (names.qp_b, params.qpb) {
            prop.set_property_int64(name, qpb as amf_int64)?;
        }
        if let (Some(name), Some(gop)) = (names.gop_pic_size, params.gop_pic_size) {
            prop.set_property_int64(name, gop as amf_int64)?;
        }
        Ok(())
    }

    /// H.264 固有のプロパティを設定する
    fn set_h264_properties(
        prop: &PropertyStorage,
        config: &EncoderConfig,
        h264: &H264EncoderConfig,
    ) -> Result<(), Error> {
        // 用途: トランスコーディング
        prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_USAGE, 0)?;

        // プロファイル
        if let Some(profile) = h264.profile {
            let profile_val: amf_int64 = match profile {
                H264Profile::Baseline => {
                    sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_BASELINE
                        as amf_int64
                }
                H264Profile::Main => {
                    sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_MAIN as amf_int64
                }
                H264Profile::High => {
                    sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_HIGH as amf_int64
                }
                H264Profile::ConstrainedBaseline => {
                    sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_CONSTRAINED_BASELINE
                        as amf_int64
                }
                H264Profile::ConstrainedHigh => {
                    sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_CONSTRAINED_HIGH
                        as amf_int64
                }
            };
            prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_PROFILE, profile_val)?;
        }

        // 出力ビットストリームのビット深度
        if let Some(color_bit_depth) = config.frame_format.output_color_bit_depth() {
            prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_COLOR_BIT_DEPTH, color_bit_depth)?;
        }

        // フレームサイズ
        prop.set_property_size(
            sys::str::AMF_VIDEO_ENCODER_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
        )?;

        prop.set_property_int64(
            sys::str::AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD,
            config.rate_control_mode.to_h264() as amf_int64,
        )?;

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

        // B フレーム (GOP 内の連続する B フレーム数)
        if let Some(pattern) = config.b_pic_pattern {
            prop.set_property_int64(
                sys::str::AMF_VIDEO_ENCODER_MAX_CONSECUTIVE_BPICTURES,
                pattern as amf_int64,
            )?;
            prop.set_property_int64(
                sys::str::AMF_VIDEO_ENCODER_B_PIC_PATTERN,
                pattern as amf_int64,
            )?;
        }

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
                HevcProfile::Main => {
                    sys::AMF_VIDEO_ENCODER_HEVC_PROFILE_ENUM_AMF_VIDEO_ENCODER_HEVC_PROFILE_MAIN
                        as amf_int64
                }
                HevcProfile::Main10 => {
                    sys::AMF_VIDEO_ENCODER_HEVC_PROFILE_ENUM_AMF_VIDEO_ENCODER_HEVC_PROFILE_MAIN_10
                        as amf_int64
                }
            };
            prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_HEVC_PROFILE, profile_val)?;
        }

        // 出力ビットストリームのビット深度
        if let Some(color_bit_depth) = config.frame_format.output_color_bit_depth() {
            prop.set_property_int64(
                sys::str::AMF_VIDEO_ENCODER_HEVC_COLOR_BIT_DEPTH,
                color_bit_depth,
            )?;
        }

        prop.set_property_size(
            sys::str::AMF_VIDEO_ENCODER_HEVC_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
        )?;

        prop.set_property_int64(
            sys::str::AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD,
            config.rate_control_mode.to_hevc() as amf_int64,
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
        av1: &Av1EncoderConfig,
    ) -> Result<(), Error> {
        prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_AV1_USAGE, 0)?;

        // プロファイル
        if let Some(profile) = av1.profile {
            let profile_val: amf_int64 = match profile {
                Av1Profile::Main => {
                    sys::AMF_VIDEO_ENCODER_AV1_PROFILE_ENUM_AMF_VIDEO_ENCODER_AV1_PROFILE_MAIN
                        as amf_int64
                }
            };
            prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_AV1_PROFILE, profile_val)?;
        }

        // 出力ビットストリームのビット深度
        if let Some(color_bit_depth) = config.frame_format.output_color_bit_depth() {
            prop.set_property_int64(
                sys::str::AMF_VIDEO_ENCODER_AV1_COLOR_BIT_DEPTH,
                color_bit_depth,
            )?;
        }

        prop.set_property_size(
            sys::str::AMF_VIDEO_ENCODER_AV1_FRAMESIZE,
            config.width as amf_int32,
            config.height as amf_int32,
        )?;

        prop.set_property_int64(
            sys::str::AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD,
            config.rate_control_mode.to_av1() as amf_int64,
        )?;

        let params = ReconfigureParams {
            target_kbps: config.target_kbps,
            max_kbps: config.max_kbps,
            qpi: config.qpi,
            qpp: config.qpp,
            qpb: config.qpb,
            gop_pic_size: config.gop_pic_size,
            ..ReconfigureParams::default()
        };
        Self::set_av1_dynamic_properties(
            prop,
            &params,
            Some((config.framerate_num, config.framerate_den)),
        )?;

        // B フレーム (GOP 内の連続する B フレーム数)
        if let Some(pattern) = config.b_pic_pattern {
            prop.set_property_int64(
                sys::str::AMF_VIDEO_ENCODER_AV1_MAX_CONSECUTIVE_BPICTURES,
                pattern as amf_int64,
            )?;
            prop.set_property_int64(
                sys::str::AMF_VIDEO_ENCODER_AV1_B_PIC_PATTERN,
                pattern as amf_int64,
            )?;
        }

        Ok(())
    }

    /// このエンコーダーが使用する `Context` を返す
    pub fn context(&self) -> Context {
        self.context.clone()
    }

    /// エンコード用の Surface を確保する
    ///
    /// 確保された Surface は呼び出し元がフレームデータを書き込んでから
    /// [`Encoder::encode`] に渡す。
    pub fn alloc_surface(&self) -> Result<Surface, Error> {
        self.context.alloc_surface(
            MemoryType::Host,
            self.surface_format,
            self.width,
            self.height,
        )
    }

    /// フレームをエンコードする
    ///
    /// `user_data` は、対応する出力フレームの [`EncodedFrame::user_data`] として
    /// ハンドラーへ渡される。
    ///
    /// 入力 1 つにつき出力 1 つを前提とし、対応が取れない入出力はハンドラーへ `Err` として
    /// 通知する。
    ///
    /// `surface` はこのエンコーダーと同じ `Context` から確保し、他のハンドルと
    /// 共有されていない Surface であること。
    /// 別の Context の Surface や、他のハンドルと共有されている Surface はエラーになる。
    ///
    /// 入出力の対応付けに `ShiguredoAmfFrameId` プロパティを使う。この名前は
    /// ライブラリの予約名であり、利用者は `surface` に設定しないこと。
    ///
    /// # Errors
    ///
    /// `finish` 済みの場合、`surface` の状態が操作に適していない場合、または
    /// AMF の操作に失敗した場合にエラーを返す。
    pub fn encode(
        &mut self,
        surface: Surface,
        options: EncodeOptions,
        user_data: H::UserData,
    ) -> Result<(), Error> {
        if !self.worker.is_running() {
            return Err(InvalidStateError::new("encoder is already finished").into());
        }

        // AMF は同じ Context に載ったオブジェクトの受け渡しを前提としているため、
        // 投入前に同じ Context のハンドルであることを確認する。
        if surface.context() != self.context {
            return Err(InvalidArgumentError::new(
                "surface does not belong to the encoder's AMF context",
            )
            .into());
        }

        // AMF コンポーネントは投入した Surface を解放するまで排他アクセスするため、
        // 他のハンドルが残っている状態では投入しない。
        if surface.is_shared() {
            return Err(InvalidStateError::new(
                "surface is shared with other handles; drop clones, planes, and property storages before submitting",
            )
            .into());
        }

        {
            // フレームタイプを強制する。SubmitInput の前にプロパティのハンドルを
            // 解放し、Surface が共有状態のまま投入されないようにする。
            let property = surface.property_storage()?;
            Self::force_picture_type(&self.codec_config, &property, options.frame_type)?;
        }

        tracing::trace!("Encoder::encode: SubmitInput");
        // `AMF_NEED_MORE_INPUT` は入力を受け付けたが、まだ出力を生成できないことを
        // 示す。B フレームでは複数の入力を必要とするため、受理として扱う。
        let data = surface.into_data()?;
        let (frame_id, status) = self.worker.submit_input(&self.component, &data)?;
        if !matches!(status, AMF_RESULT::AMF_OK | AMF_RESULT::AMF_NEED_MORE_INPUT) {
            return Err(AmfError::new(status, "AMFComponent::SubmitInput").into());
        }

        // AMF が入力を受け付けたため、終了時には Drain が必要になる。
        self.drain_required = true;

        // ワーカースレッドに user_data の登録コマンドを送信する
        self.worker.register_user_data(frame_id, user_data)?;

        Ok(())
    }

    /// `frame_type` の指定から優先度の最も高い 1 つを返す
    ///
    /// `IDR`、`I`、`P` の順に優先する。該当するフラグがない場合は `None` を返す。
    fn resolve_frame_type(frame_type_value: u16) -> Option<u16> {
        if frame_type_value & frame_type::IDR != 0 {
            Some(frame_type::IDR)
        } else if frame_type_value & frame_type::I != 0 {
            Some(frame_type::I)
        } else if frame_type_value & frame_type::P != 0 {
            Some(frame_type::P)
        } else {
            None
        }
    }

    /// 指定されたフレームタイプを AMF に強制する
    fn force_picture_type(
        codec_config: &CodecConfig,
        prop: &PropertyStorage,
        frame_type_value: u16,
    ) -> Result<(), Error> {
        let Some(resolved) = Self::resolve_frame_type(frame_type_value) else {
            return Ok(());
        };

        // AV1 には P を強制する値がない。
        if resolved == frame_type::P && matches!(codec_config, CodecConfig::Av1(_)) {
            return Ok(());
        }

        match codec_config {
            CodecConfig::H264(_) => {
                let value: amf_int64 = match resolved {
                    frame_type::IDR => {
                        sys::AMF_VIDEO_ENCODER_PICTURE_TYPE_ENUM_AMF_VIDEO_ENCODER_PICTURE_TYPE_IDR
                            .into()
                    }
                    frame_type::I => {
                        sys::AMF_VIDEO_ENCODER_PICTURE_TYPE_ENUM_AMF_VIDEO_ENCODER_PICTURE_TYPE_I
                            .into()
                    }
                    _ => sys::AMF_VIDEO_ENCODER_PICTURE_TYPE_ENUM_AMF_VIDEO_ENCODER_PICTURE_TYPE_P
                        .into(),
                };
                prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_FORCE_PICTURE_TYPE, value)?;
                if resolved == frame_type::IDR {
                    prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_INSERT_SPS, 1)?;
                    prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_INSERT_PPS, 1)?;
                }
            }
            CodecConfig::Hevc(_) => {
                let value: amf_int64 = match resolved {
                    frame_type::IDR => {
                        sys::AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_IDR.into()
                    }
                    frame_type::I => {
                        sys::AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_I.into()
                    }
                    _ => {
                        sys::AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_PICTURE_TYPE_P.into()
                    }
                };
                prop.set_property_int64(
                    sys::str::AMF_VIDEO_ENCODER_HEVC_FORCE_PICTURE_TYPE,
                    value,
                )?;
                if resolved == frame_type::IDR {
                    prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_HEVC_INSERT_HEADER, 1)?;
                }
            }
            CodecConfig::Av1(_) => {
                let value: amf_int64 = match resolved {
                    frame_type::IDR => {
                        sys::AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE_KEY.into()
                    }
                    _ => {
                        sys::AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE_INTRA_ONLY.into()
                    }
                };
                prop.set_property_int64(sys::str::AMF_VIDEO_ENCODER_AV1_FORCE_FRAME_TYPE, value)?;
                if resolved == frame_type::IDR {
                    prop.set_property_int64(
                        sys::str::AMF_VIDEO_ENCODER_AV1_FORCE_INSERT_SEQUENCE_HEADER,
                        1,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// 残りのフレームを出力させ、ハンドラーへの通知が完了するまで待つ。
    ///
    /// 入力が受理済みの場合はワーカーに Drain と出力の回収をさせる。受理済みの
    /// 入力がない場合はワーカーを停止するだけである。出力の回収中に発生した
    /// 終端エラーはハンドラーへ通知される。
    ///
    /// # Errors
    ///
    /// `finish` 済みの場合、残りの出力を回収できなかった場合、または
    /// ハンドラーが panic した場合にエラーを返す。エラーを返した後の操作は
    /// `Drop` 以外を保証しない。
    pub fn finish(&mut self) -> Result<(), Error> {
        self.worker.finish(self.drain_required)?;
        self.drain_required = false;
        Ok(())
    }
}

// SAFETY:
// Drop 内でのみ component を解放する。ワーカースレッドは Drop より先に停止させる。
impl<H: EncodeHandler> Drop for Encoder<H> {
    fn drop(&mut self) {
        // ワーカースレッドに Stop を送り、キューにあるコマンドを処理してから停止する。
        // Stop を処理した時点で残っている入出力は通知せずに破棄される。
        self.worker.shutdown();

        // 戻り値を返せない後始末であるため、失敗は記録だけ行う。
        let result = self.component.terminate();
        if result != AMF_RESULT::AMF_OK {
            tracing::error!("Encoder::drop: AMFComponent::Terminate failed: {result:?}");
        }
    }
}

/// 出力バッファを検証し、ピクチャタイプと一緒に返す。
///
/// # Errors
///
/// 出力バッファのサイズが 0 の場合、ネイティブポインタが null の場合、または
/// ピクチャタイプを取得できない場合にエラーを返す。
fn validate_encoded_output(
    buffer: Buffer,
    codec_config: &CodecConfig,
) -> Result<(Buffer, PictureType), Error> {
    let buf_size = buffer.get_size();
    if buf_size == 0 {
        return Err(InternalError::new("output buffer size is 0").into());
    }
    // エンコーダーの出力バッファはホストメモリとは限らないため、メモリ種別ではなく
    // GetNative がデータへのポインタを返すことだけを確認する。
    let buf_native = buffer.get_native() as *const u8;
    if buf_native.is_null() {
        return Err(InternalError::new("output buffer native pointer is null").into());
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
    let property = buffer.property_storage()?;
    // AMF の出力フレーム種別は amf_int64 のプロパティとして定義されている。
    let type_val = property.get_property_int64(prop_name)?;
    Ok(picture_type_from_output_value(codec_config, type_val))
}

/// コーデックごとの出力フレーム種別値を [`PictureType`] に変換する
///
/// 出力フレーム種別の値は、AMF の出力データ型または出力フレーム種別の enum で
/// 定義される。enum に定義されていない値は [`PictureType::Unknown`] を返す。
fn picture_type_from_output_value(codec_config: &CodecConfig, value: amf_int64) -> PictureType {
    // AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM の値
    const H264_IDR: amf_int64 =
        sys::AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_IDR
            as amf_int64;
    const H264_I: amf_int64 =
        sys::AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_I
            as amf_int64;
    const H264_P: amf_int64 =
        sys::AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_P
            as amf_int64;
    const H264_B: amf_int64 =
        sys::AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_B
            as amf_int64;

    // AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM の値
    const HEVC_IDR: amf_int64 =
        sys::AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_IDR
            as amf_int64;
    const HEVC_I: amf_int64 =
        sys::AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_I
            as amf_int64;
    const HEVC_P: amf_int64 =
        sys::AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_P
            as amf_int64;

    // AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM の値
    const AV1_KEY: amf_int64 =
        sys::AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_KEY
            as amf_int64;
    const AV1_INTRA_ONLY: amf_int64 =
        sys::AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_INTRA_ONLY
            as amf_int64;
    const AV1_INTER: amf_int64 =
        sys::AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_INTER
            as amf_int64;
    const AV1_SWITCH: amf_int64 =
        sys::AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_SWITCH
            as amf_int64;
    const AV1_SHOW_EXISTING: amf_int64 =
        sys::AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_SHOW_EXISTING
            as amf_int64;

    match codec_config {
        CodecConfig::H264(_) => match value {
            H264_IDR => PictureType::Idr,
            H264_I => PictureType::I,
            H264_P => PictureType::P,
            H264_B => PictureType::B,
            _ => PictureType::Unknown,
        },
        CodecConfig::Hevc(_) => match value {
            HEVC_IDR => PictureType::Idr,
            HEVC_I => PictureType::I,
            HEVC_P => PictureType::P,
            _ => PictureType::Unknown,
        },
        CodecConfig::Av1(_) => match value {
            AV1_KEY => PictureType::Idr,
            AV1_INTRA_ONLY => PictureType::I,
            AV1_INTER => PictureType::P,
            // SWITCH はストリームの切り替え、SHOW_EXISTING は既存フレームの
            // 再表示に使うフレームであり、IDR / I / P / B のいずれにも対応しないため
            // Unknown にする。
            AV1_SWITCH | AV1_SHOW_EXISTING => PictureType::Unknown,
            _ => PictureType::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の AV1 コーデック設定を返す
    fn av1_codec() -> CodecConfig {
        CodecConfig::Av1(Av1EncoderConfig {
            profile: Some(Av1Profile::Main),
        })
    }

    /// AV1 の出力フレーム種別のうち再現できない値を変換できることを確認する
    ///
    /// `INTRA_ONLY` / `SWITCH` / `SHOW_EXISTING` と未知の値は、このライブラリの
    /// 公開 API で生成を制御できないため、変換処理を直接検証する。
    /// `KEY` と `INTER` は実機テストで検証する。
    #[test]
    fn test_picture_type_from_output_value_av1_edge_cases() {
        let codec = av1_codec();
        assert_eq!(
            picture_type_from_output_value(&codec, 1),
            PictureType::I,
            "INTRA_ONLY は I に変換されること"
        );
        assert_eq!(
            picture_type_from_output_value(&codec, 3),
            PictureType::Unknown,
            "SWITCH は Unknown に変換されること"
        );
        assert_eq!(
            picture_type_from_output_value(&codec, 4),
            PictureType::Unknown,
            "SHOW_EXISTING は Unknown に変換されること"
        );
        assert_eq!(
            picture_type_from_output_value(&codec, 5),
            PictureType::Unknown,
            "未定義の値は Unknown に変換されること"
        );
        assert_eq!(
            picture_type_from_output_value(&codec, -1),
            PictureType::Unknown,
            "負の値は Unknown に変換されること"
        );
    }
}
