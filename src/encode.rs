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
use crate::query_output::{DrainStatus, QueryOutputKind, classify_query_output};
use crate::sys::{
    self, AMF_MEMORY_TYPE, AMF_RESULT, AMF_SECOND, AMF_SURFACE_FORMAT, AMFBuffer, AMFData,
    AMFVariantStruct, amf_int32, amf_int64, amf_pts,
};
use crate::types::{Av1Profile, FrameFormat, H264Profile, HevcProfile};

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

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

    /// 指定解像度でのフレームサイズ（バイト数）を計算する
    ///
    /// オーバーフローした場合は `None` を返す。
    pub fn frame_size(self, width: usize, height: usize) -> Option<usize> {
        let pixels = width.checked_mul(height)?;
        match self {
            // YUV 4:2:0 8bit 系は 1 ピクセルあたり 1.5 バイト
            FrameFormat::Nv12 | FrameFormat::Yv12 | FrameFormat::I420 => {
                pixels.checked_mul(3).map(|v| v / 2)
            }
            // Packed 32bit 系は 1 ピクセルあたり 4 バイト
            FrameFormat::Bgra
            | FrameFormat::Argb
            | FrameFormat::Rgba
            | FrameFormat::Ayuv
            | FrameFormat::Y410 => pixels.checked_mul(4),
            // Packed YUV 4:2:2 8bit 系は 1 ピクセルあたり 2 バイト
            FrameFormat::Yuy2 | FrameFormat::Uyvy => pixels.checked_mul(2),
            // Semi-Planar YUV 4:2:0 16bit 系は 1 ピクセルあたり 3 バイト
            FrameFormat::P010 | FrameFormat::P012 | FrameFormat::P016 => pixels.checked_mul(3),
            // Packed YUV 4:2:2 10bit 系は 1 ピクセルあたり 4 バイト
            FrameFormat::Y210 => pixels.checked_mul(4),
            // Packed YUV 4:4:4 16bit 系は 1 ピクセルあたり 8 バイト
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
// frame_type 定数
// ---------------------------------------------------------------------------
/// 強制するフレームタイプを表すビットフラグ
///
/// [`EncodeOptions::frame_type`] に指定する。AMF のピクチャタイプ enum とは
/// 独立したビットフラグである。`IDR` / `I` / `P` が指定された場合は `IDR`、`I`、
/// `P` の順に優先して 1 つを AMF の強制ピクチャタイプへ変換する。
///
/// `B` と `REF`、未定義のビット、利用するコーデックが対応しないフラグは無視する。
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

// ---------------------------------------------------------------------------
// ワーカースレッド用コマンド
// ---------------------------------------------------------------------------

enum WorkerCommand<T> {
    Submit(T),
    Finish(mpsc::SyncSender<()>),
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
    pub fn new(context: Context, config: EncoderConfig, handler: H) -> Result<Self, Error> {
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

        // HEVC には B フレーム用の QP プロパティがないため、qpb は指定できない。
        if matches!(&config.codec, CodecConfig::Hevc(_)) && config.qpb.is_some() {
            return Err(Error::new_custom(
                "Encoder::new",
                "qpb is not supported for HEVC",
            ));
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
        Err(Error::new_custom(
            "Encoder::new",
            &format!(
                "HEVC 10-bit input requires HevcProfile::Main10: {profile} with {:?} (set HevcProfile::Main10)",
                config.frame_format
            ),
        ))
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
    /// `params` の値が不正な場合、または AMF のプロパティを設定できない場合に
    /// エラーを返す。コーデックが対応していない項目を指定した場合も含む。
    pub fn reconfigure(&mut self, params: ReconfigureParams) -> Result<(), Error> {
        // HEVC には B フレーム用の QP プロパティがない。また AMF の HEVC では
        // GOP サイズが初期化前の静的プロパティであり、H.264 の IDRPeriod や
        // AV1 の GOPSize と異なり初期化後には変更できない。再設定では指定できない。
        if let CodecConfig::Hevc(_) = &self.codec_config {
            if params.qpb.is_some() {
                return Err(Error::new_custom(
                    "Encoder::reconfigure",
                    "qpb is not supported for HEVC",
                ));
            }
            if params.gop_pic_size.is_some() {
                return Err(Error::new_custom(
                    "Encoder::reconfigure",
                    "gop_pic_size cannot be changed for HEVC",
                ));
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

        // レート制御
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
            qpb: config.qpb,
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
            AMF_MEMORY_TYPE::AMF_MEMORY_HOST,
            self.surface_format,
            self.width,
            self.height,
        )
    }

    /// フレームをエンコードする
    ///
    /// `user_data` はエンコード完了時にハンドラーへ渡される。
    ///
    /// `surface` はこのエンコーダーと同じ `Context` から確保したものでなければ
    /// ならない。別の Context から確保した Surface を渡した場合はエラーを返す。
    /// `surface` が他のハンドルと共有されている場合もエラーを返す。
    ///
    /// # Errors
    ///
    /// `surface` の状態が操作に適していない場合、または AMF への入力に失敗した
    /// 場合にエラーを返す。
    pub fn encode(
        &mut self,
        surface: Surface,
        options: &EncodeOptions,
        user_data: H::UserData,
    ) -> Result<(), Error> {
        // AMF は同じ Context に載ったオブジェクトの受け渡しを前提としているため、
        // 投入前に同じ Context のハンドルであることを確認する。
        if surface.context() != self.context {
            return Err(Error::new_custom(
                "Encoder::encode",
                "surface does not belong to the encoder's AMF context",
            ));
        }

        // AMF コンポーネントは投入した Surface を解放するまで排他アクセスするため、
        // 他のハンドルが残っている状態では投入しない。
        if surface.is_shared() {
            return Err(Error::new_custom(
                "Encoder::encode",
                "surface is shared with other handles; drop clones, planes, and property storages before submitting",
            ));
        }

        // PTS を設定する
        let pts = (self.frame_count * self.framerate_den * AMF_SECOND as u64 / self.framerate_num)
            as amf_pts;
        surface.set_pts(pts);

        // フレームタイプの強制
        self.force_picture_type(&surface, options.frame_type)?;

        tracing::trace!("Encoder::encode: SubmitInput");
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
                    tracing::trace!("Encoder::SubmitInput: retry={retry}, result={result:?}");
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
            .expect("cmd_tx must be set before encode")
            .send(WorkerCommand::Submit(user_data))
            .map_err(|_| Error::new_custom("Encoder::encode", "worker thread terminated"))?;

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
    fn force_picture_type(&self, surface: &Surface, frame_type_value: u16) -> Result<(), Error> {
        let Some(resolved) = Self::resolve_frame_type(frame_type_value) else {
            return Ok(());
        };

        // AV1 には P を強制する値がない。
        if resolved == frame_type::P && matches!(&self.codec_config, CodecConfig::Av1(_)) {
            return Ok(());
        }

        let prop = surface.property_storage()?;
        match &self.codec_config {
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
    /// 処理中に発生したエラーはハンドラーへ通知される。ワーカーが既に終了している
    /// 場合や Drain に失敗した場合は `Err` を返す。
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
            .expect("cmd_tx must be set before finish")
            .send(WorkerCommand::Finish(tx))
            .map_err(|_| Error::new_custom("Encoder::finish", "worker thread terminated"))?;

        // ワーカーの完了を待つ
        rx.recv()
            .map_err(|_| Error::new_custom("Encoder::finish", "worker thread terminated"))?;

        Ok(())
    }
}

// SAFETY:
// Drop 内でのみ component を解放する。ワーカースレッドは Drop より先に停止させる。
impl<H: EncodeHandler> Drop for Encoder<H> {
    fn drop(&mut self) {
        // ワーカースレッドを停止する (cmd_tx を drop → チャネル切断 → ワーカー終了)
        self.cmd_tx = None;

        if let Some(handle) = self.poll_thread.take() {
            let _ = handle.join();
        }

        let _ = self.component.terminate();
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
    let mut output_buffer: VecDeque<Result<(Buffer, PictureType), Error>> = VecDeque::new();
    let mut finish: Option<mpsc::SyncSender<()>> = None;

    loop {
        if finish.is_some() {
            // Drain 後は、AMF_EOF が返るか、保持している入出力がなくなるまで
            // ポーリングする。
            match drain_output(
                &mut output_buffer,
                &mut pending,
                &mut handler,
                &component,
                &codec_config,
            ) {
                Ok(status) => {
                    if status == DrainStatus::Eof {
                        flush_buffered_outputs(&mut output_buffer, &mut handler);
                        report_unmatched_inputs(&mut pending, &mut handler, "Encoder::finish");
                        break;
                    }
                    if pending.is_empty() && output_buffer.is_empty() {
                        break;
                    }
                }
                Err(error) => {
                    // 終端エラーでは AMF_EOF を待たず、取得済みの出力と残りの入力を
                    // 通知してワーカーを終了する。
                    notify_terminal_error(error, &mut output_buffer, &mut pending, &mut handler);
                    break;
                }
            }

            std::thread::sleep(Duration::from_millis(1));
            continue;
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
                    if let Err(error) = drain_output(
                        &mut output_buffer,
                        &mut pending,
                        &mut handler,
                        &component,
                        &codec_config,
                    ) {
                        // 通常のエンコード中でも終端エラーは起き得るため、同じように
                        // 取得済みの出力と残りの入力を通知して終了する。
                        notify_terminal_error(
                            error,
                            &mut output_buffer,
                            &mut pending,
                            &mut handler,
                        );
                        break;
                    }
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

/// `QueryOutput` から取得できた出力を投入順に対応する入力と組み合わせ、ハンドラーへ
/// 通知する。
///
/// 出力が尽きた場合は [`DrainStatus::NotReady`]、ドレインが完了した場合は
/// [`DrainStatus::Eof`] を返す。終端エラーの場合は `Err` を返す。終端エラー時に
/// 通知できなかった出力は保持する。
fn drain_output<H: EncodeHandler>(
    output_buffer: &mut VecDeque<Result<(Buffer, PictureType), Error>>,
    pending: &mut VecDeque<H::UserData>,
    handler: &mut H,
    component: &Component,
    codec_config: &CodecConfig,
) -> Result<DrainStatus, Error> {
    let context = component.context();
    let status = loop {
        let mut data: *mut AMFData = ptr::null_mut();
        let result = unsafe { component.query_output(&mut data) };
        tracing::trace!("worker: QueryOutput result={result:?}");
        let kind = classify_query_output(result, !data.is_null());
        // QueryOutput が返した参照は呼び出し元が所有する。
        if kind != QueryOutputKind::Output && !data.is_null() {
            let vtbl = unsafe { &*(*data).pVtbl };
            if let Some(release) = vtbl.Release {
                unsafe { release(data) };
            }
        }
        match kind {
            QueryOutputKind::Output => {
                output_buffer.push_back(extract_encoded_output(data, codec_config, &context));
            }
            QueryOutputKind::NoOutput => {
                break DrainStatus::NotReady;
            }
            QueryOutputKind::Eof => {
                break DrainStatus::Eof;
            }
            QueryOutputKind::TerminalError => {
                match_outputs(output_buffer, pending, handler);
                return Err(Error::from_amf(result, "AMFComponent::QueryOutput"));
            }
        }
    };

    match_outputs(output_buffer, pending, handler);
    Ok(status)
}

/// 取得済みの出力を投入順に対応する入力と組み合わせ、ハンドラーへ通知する。
fn match_outputs<H: EncodeHandler>(
    output_buffer: &mut VecDeque<Result<(Buffer, PictureType), Error>>,
    pending: &mut VecDeque<H::UserData>,
    handler: &mut H,
) {
    while !output_buffer.is_empty() && !pending.is_empty() {
        let output = output_buffer.pop_front().expect("front output must exist");
        let user_data = pending.pop_front().expect("front pending must exist");
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

/// 保持している出力をすべてハンドラーへ通知する。
///
/// 対応する入力がない出力は `Err` として通知する。ドレイン完了時と終端エラー時に
/// 呼ぶ。
fn flush_buffered_outputs<H: EncodeHandler>(
    output_buffer: &mut VecDeque<Result<(Buffer, PictureType), Error>>,
    handler: &mut H,
) {
    while let Some(output) = output_buffer.pop_front() {
        match output {
            Ok(_) => handler.on_encoded(Err(Error::new_custom(
                "Encoder::worker",
                "no submitted frame for encoded output",
            )
            .into())),
            Err(error) => handler.on_encoded(Err(error.into())),
        }
    }
}

/// 出力が得られなかった入力を、`function` を発生元とする `Err` として通知する。
fn report_unmatched_inputs<H: EncodeHandler>(
    pending: &mut VecDeque<H::UserData>,
    handler: &mut H,
    function: &'static str,
) {
    for _ in pending.drain(..) {
        handler.on_encoded(Err(Error::new_custom(
            function,
            "encoder did not output a frame for submitted input",
        )
        .into()));
    }
}

/// 終端エラー発生時に、保持している出力、原因のエラー、出力が得られなかった入力の
/// 順にハンドラーへ通知する。
fn notify_terminal_error<H: EncodeHandler>(
    error: Error,
    output_buffer: &mut VecDeque<Result<(Buffer, PictureType), Error>>,
    pending: &mut VecDeque<H::UserData>,
    handler: &mut H,
) {
    flush_buffered_outputs(output_buffer, handler);
    handler.on_encoded(Err(error.into()));
    report_unmatched_inputs(pending, handler, "Encoder::worker");
}

/// 出力バッファとピクチャタイプを返す。
///
/// 出力バッファを返せない場合は `Err` を返す。
fn extract_encoded_output(
    data: *mut AMFData,
    codec_config: &CodecConfig,
    context: &Context,
) -> Result<(Buffer, PictureType), Error> {
    let buffer = data as *mut AMFBuffer;
    if buffer.is_null() {
        return Err(Error::new_custom(
            "extract_encoded_output",
            "buffer is null",
        ));
    }
    // エンコーダーの出力バッファは、エンコーダーが作成された Context から確保されている
    let buffer = unsafe { Buffer::from_raw(buffer, context.clone()) }?;
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

    // AMF の出力フレーム種別は amf_int64 のプロパティとして定義されている。
    let type_val = unsafe { var.__bindgen_anon_1.int64Value };
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
