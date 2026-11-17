//! コーデック情報の照会

use std::ffi::CStr;

use crate::AmfLibrary;
use crate::Error;
use crate::amf::{Caps, Context, IoCaps as RawIoCaps};
use crate::sys;
use crate::types::{Av1Profile, FrameFormat, H264Profile, HevcProfile};

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

/// コーデック種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodecType {
    /// H.264 / AVC コーデック
    H264,
    /// H.265 / HEVC コーデック
    Hevc,
    /// AV1 コーデック
    Av1,
}

impl VideoCodecType {
    /// すべてのコーデック種別を返す
    fn all() -> &'static [Self] {
        &[Self::H264, Self::Hevc, Self::Av1]
    }
}

/// AMF が報告するアクセラレーション種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelerationType {
    /// 現在のハードウェアでは非対応
    NotSupported,
    /// 固定機能ハードウェアによるアクセラレーション
    Hardware,
    /// プログラマブルな GPU (シェーダー) によるアクセラレーション
    Gpu,
    /// ソフトウェアによる処理
    Software,
    /// AMF が追加した未知の種別
    Unknown(i32),
}

impl AccelerationType {
    /// `AMF_ACCELERATION_TYPE` の値から変換する
    fn from_amf(value: sys::AMF_ACCELERATION_TYPE) -> Self {
        match value {
            sys::AMF_ACCELERATION_TYPE_AMF_ACCEL_NOT_SUPPORTED => Self::NotSupported,
            sys::AMF_ACCELERATION_TYPE_AMF_ACCEL_HARDWARE => Self::Hardware,
            sys::AMF_ACCELERATION_TYPE_AMF_ACCEL_GPU => Self::Gpu,
            sys::AMF_ACCELERATION_TYPE_AMF_ACCEL_SOFTWARE => Self::Software,
            other => Self::Unknown(other),
        }
    }
}

/// コーデックの最大プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoProfile {
    /// H.264 プロファイル
    H264(H264Profile),
    /// HEVC プロファイル
    Hevc(HevcProfile),
    /// AV1 プロファイル
    Av1(Av1Profile),
    /// 本クレートが型として持たないプロファイル
    Unknown(i64),
}

impl VideoProfile {
    /// AMF のプロファイル値から変換する
    ///
    /// H.264 の `AMF_VIDEO_ENCODER_PROFILE_UNKNOWN` (0) はプロファイル不明として
    /// `None` を返す。それ以外の未知の値は [`VideoProfile::Unknown`] に生値を保持する。
    fn from_amf(codec: VideoCodecType, value: sys::amf_int64) -> Option<Self> {
        match codec {
            VideoCodecType::H264 => {
                if value
                    == sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_UNKNOWN
                        as sys::amf_int64
                {
                    return None;
                }
                if value
                    == sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_BASELINE
                        as sys::amf_int64
                {
                    Some(Self::H264(H264Profile::Baseline))
                } else if value
                    == sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_MAIN
                        as sys::amf_int64
                {
                    Some(Self::H264(H264Profile::Main))
                } else if value
                    == sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_HIGH
                        as sys::amf_int64
                {
                    Some(Self::H264(H264Profile::High))
                } else if value
                    == sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_CONSTRAINED_BASELINE
                        as sys::amf_int64
                {
                    Some(Self::H264(H264Profile::ConstrainedBaseline))
                } else if value
                    == sys::AMF_VIDEO_ENCODER_PROFILE_ENUM_AMF_VIDEO_ENCODER_PROFILE_CONSTRAINED_HIGH
                        as sys::amf_int64
                {
                    Some(Self::H264(H264Profile::ConstrainedHigh))
                } else {
                    Some(Self::Unknown(value))
                }
            }
            VideoCodecType::Hevc => {
                if value
                    == sys::AMF_VIDEO_ENCODER_HEVC_PROFILE_ENUM_AMF_VIDEO_ENCODER_HEVC_PROFILE_MAIN
                        as sys::amf_int64
                {
                    Some(Self::Hevc(HevcProfile::Main))
                } else if value
                    == sys::AMF_VIDEO_ENCODER_HEVC_PROFILE_ENUM_AMF_VIDEO_ENCODER_HEVC_PROFILE_MAIN_10
                        as sys::amf_int64
                {
                    Some(Self::Hevc(HevcProfile::Main10))
                } else {
                    Some(Self::Unknown(value))
                }
            }
            VideoCodecType::Av1 => {
                if value
                    == sys::AMF_VIDEO_ENCODER_AV1_PROFILE_ENUM_AMF_VIDEO_ENCODER_AV1_PROFILE_MAIN
                        as sys::amf_int64
                {
                    Some(Self::Av1(Av1Profile::Main))
                } else {
                    Some(Self::Unknown(value))
                }
            }
        }
    }
}

/// AMF が報告する surface format
///
/// `FrameFormat` に写像できる値は [`SurfaceFormat::Known`] に、写像できない値は
/// [`SurfaceFormat::Unknown`] に AMF の生値を保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceFormat {
    /// `FrameFormat` に写像できる形式
    Known(FrameFormat),
    /// `FrameFormat` に写像できない形式
    Unknown(i32),
}

impl SurfaceFormat {
    /// `AMF_SURFACE_FORMAT` の値から変換する
    fn from_amf(format: sys::AMF_SURFACE_FORMAT) -> Self {
        if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12 {
            Self::Known(FrameFormat::Nv12)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YV12 {
            Self::Known(FrameFormat::Yv12)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YUV420P {
            Self::Known(FrameFormat::I420)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_BGRA {
            Self::Known(FrameFormat::Bgra)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_ARGB {
            Self::Known(FrameFormat::Argb)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_RGBA {
            Self::Known(FrameFormat::Rgba)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YUY2 {
            Self::Known(FrameFormat::Yuy2)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_UYVY {
            Self::Known(FrameFormat::Uyvy)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P010 {
            Self::Known(FrameFormat::P010)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P012 {
            Self::Known(FrameFormat::P012)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P016 {
            Self::Known(FrameFormat::P016)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y210 {
            Self::Known(FrameFormat::Y210)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_AYUV {
            Self::Known(FrameFormat::Ayuv)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y410 {
            Self::Known(FrameFormat::Y410)
        } else if format == sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y416 {
            Self::Known(FrameFormat::Y416)
        } else {
            Self::Unknown(format.0 as i32)
        }
    }
}

/// AMF が報告する memory type
///
/// Linux で使う値だけを型にし、それ以外は [`MemoryType::Unknown`] に生値を保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryType {
    /// ホストメモリ
    Host,
    /// OpenCL メモリ
    OpenCl,
    /// OpenGL メモリ
    OpenGl,
    /// X Video メモリ
    Xv,
    /// Vulkan メモリ
    Vulkan,
    /// 本クレートが型として持たない memory type
    Unknown(i32),
}

impl MemoryType {
    /// `AMF_MEMORY_TYPE` の値から変換する
    fn from_amf(memory_type: sys::AMF_MEMORY_TYPE) -> Self {
        if memory_type == sys::AMF_MEMORY_TYPE::AMF_MEMORY_HOST {
            Self::Host
        } else if memory_type == sys::AMF_MEMORY_TYPE::AMF_MEMORY_OPENCL {
            Self::OpenCl
        } else if memory_type == sys::AMF_MEMORY_TYPE::AMF_MEMORY_OPENGL {
            Self::OpenGl
        } else if memory_type == sys::AMF_MEMORY_TYPE::AMF_MEMORY_XV {
            Self::Xv
        } else if memory_type == sys::AMF_MEMORY_TYPE::AMF_MEMORY_VULKAN {
            Self::Vulkan
        } else {
            Self::Unknown(memory_type.0 as i32)
        }
    }
}

/// 対応する surface format と native フラグ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceFormatCapability {
    /// surface format
    pub format: SurfaceFormat,
    /// AMF がこの形式を native と報告しているか
    pub native: bool,
}

/// 対応する memory type と native フラグ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryTypeCapability {
    /// memory type
    pub memory_type: MemoryType,
    /// AMF がこの memory type を native と報告しているか
    pub native: bool,
}

/// 入出力の能力情報
///
/// 解像度範囲、垂直アライメント、対応する surface format / memory type、インターレース対応を表す。
/// `formats` と `memory_types` は AMF が返した順序を維持する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoCapabilities {
    /// 最小幅
    pub min_width: i32,
    /// 最大幅
    pub max_width: i32,
    /// 最小高さ
    pub min_height: i32,
    /// 最大高さ
    pub max_height: i32,
    /// 垂直アライメント (行数の倍数制約)
    pub vertical_alignment: i32,
    /// 対応する surface format
    pub formats: Vec<SurfaceFormatCapability>,
    /// 対応する memory type
    pub memory_types: Vec<MemoryTypeCapability>,
    /// インターレースに対応しているか
    pub interlaced: bool,
}

/// AMF のケーパビリティプロパティ
///
/// この型は AMF が返した値を確認するためのものであり、値の意味や存在は
/// AMF のバージョンとドライバーに依存する。動作の制御には
/// [`EncodingInfo`] / [`DecodingInfo`] の各フィールドを使う。
/// 各プロパティは AMF が返した順序で並ぶ。
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    /// プロパティ名
    pub name: String,
    /// プロパティ値
    pub value: Value,
}

/// AMF の variant 値
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 空の値
    Empty,
    /// bool 値
    Bool(bool),
    /// int64 値
    Int64(i64),
    /// double 値
    Double(f64),
    /// 矩形
    Rect {
        /// 矩形の左端
        left: i32,
        /// 矩形の上端
        top: i32,
        /// 矩形の右端
        right: i32,
        /// 矩形の下端
        bottom: i32,
    },
    /// サイズ
    Size {
        /// サイズの幅
        width: i32,
        /// サイズの高さ
        height: i32,
    },
    /// 点
    Point {
        /// 点の X 座標
        x: i32,
        /// 点の Y 座標
        y: i32,
    },
    /// レート
    Rate {
        /// レートの分子
        num: u32,
        /// レートの分母
        den: u32,
    },
    /// 比率
    Ratio {
        /// 比率の分子
        num: u32,
        /// 比率の分母
        den: u32,
    },
    /// 色
    Color {
        /// 赤
        r: u8,
        /// 緑
        g: u8,
        /// 青
        b: u8,
        /// アルファ
        a: u8,
    },
    /// 文字列
    String(String),
    /// ワイド文字列
    WString(String),
    /// AMF のインターフェース値
    ///
    /// インターフェースのポインタは公開せず、値が存在することだけを表す。
    Interface,
    /// float 値
    Float(f32),
    /// float のサイズ
    FloatSize {
        /// float サイズの幅
        width: f32,
        /// float サイズの高さ
        height: f32,
    },
    /// float の 2 次元点
    FloatPoint2D {
        /// float 2D 点の X 座標
        x: f32,
        /// float 2D 点の Y 座標
        y: f32,
    },
    /// float の 3 次元点
    FloatPoint3D {
        /// float 3D 点の X 座標
        x: f32,
        /// float 3D 点の Y 座標
        y: f32,
        /// float 3D 点の Z 座標
        z: f32,
    },
    /// float の 4 次元ベクトル
    FloatVector4D {
        /// float 4D ベクトルの X 成分
        x: f32,
        /// float 4D ベクトルの Y 成分
        y: f32,
        /// float 4D ベクトルの Z 成分
        z: f32,
        /// float 4D ベクトルの W 成分
        w: f32,
    },
    /// 本クレートが型として持たない variant
    Unknown {
        /// AMF の variant 型 ID
        type_id: i32,
        /// variant の共用体の生値
        raw: [u8; 16],
    },
}

impl Value {
    /// `AMFVariantStruct` の値から変換する
    fn from_variant(value: &sys::AMFVariantStruct) -> Self {
        unsafe {
            if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_EMPTY {
                Self::Empty
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_BOOL {
                Self::Bool(value.__bindgen_anon_1.boolValue != 0)
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_INT64 {
                Self::Int64(value.__bindgen_anon_1.int64Value)
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_DOUBLE {
                Self::Double(value.__bindgen_anon_1.doubleValue)
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_RECT {
                let rect = value.__bindgen_anon_1.rectValue;
                Self::Rect {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_SIZE {
                let size = value.__bindgen_anon_1.sizeValue;
                Self::Size {
                    width: size.width,
                    height: size.height,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_POINT {
                let point = value.__bindgen_anon_1.pointValue;
                Self::Point {
                    x: point.x,
                    y: point.y,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_RATE {
                let rate = value.__bindgen_anon_1.rateValue;
                Self::Rate {
                    num: rate.num,
                    den: rate.den,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_RATIO {
                let ratio = value.__bindgen_anon_1.ratioValue;
                Self::Ratio {
                    num: ratio.num,
                    den: ratio.den,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_COLOR {
                // AMFColor の rgba は r, g, b, a の順に並ぶ 32bit 値として扱う。
                // 対象の Linux x86_64 はリトルエンディアンなので r が最下位バイトになる。
                let rgba = value.__bindgen_anon_1.colorValue.__bindgen_anon_1.rgba;
                Self::Color {
                    r: (rgba & 0xff) as u8,
                    g: ((rgba >> 8) & 0xff) as u8,
                    b: ((rgba >> 16) & 0xff) as u8,
                    a: ((rgba >> 24) & 0xff) as u8,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_STRING {
                let ptr = value.__bindgen_anon_1.stringValue;
                if ptr.is_null() {
                    Self::String(String::new())
                } else {
                    Self::String(CStr::from_ptr(ptr).to_string_lossy().into_owned())
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_WSTRING {
                let ptr = value.__bindgen_anon_1.wstringValue;
                if ptr.is_null() {
                    Self::WString(String::new())
                } else {
                    let mut len = 0;
                    while *ptr.add(len) != 0 {
                        len += 1;
                    }
                    let s = std::slice::from_raw_parts(ptr, len);
                    Self::WString(sys::from_wstring(s))
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_INTERFACE {
                Self::Interface
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_FLOAT {
                Self::Float(value.__bindgen_anon_1.floatValue)
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_FLOAT_SIZE {
                let size = value.__bindgen_anon_1.floatSizeValue;
                Self::FloatSize {
                    width: size.width,
                    height: size.height,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_FLOAT_POINT2D {
                let point = value.__bindgen_anon_1.floatPoint2DValue;
                Self::FloatPoint2D {
                    x: point.x,
                    y: point.y,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_FLOAT_POINT3D {
                let point = value.__bindgen_anon_1.floatPoint3DValue;
                Self::FloatPoint3D {
                    x: point.x,
                    y: point.y,
                    z: point.z,
                }
            } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_FLOAT_VECTOR4D {
                let vector = value.__bindgen_anon_1.floatVector4DValue;
                Self::FloatVector4D {
                    x: vector.x,
                    y: vector.y,
                    z: vector.z,
                    w: vector.w,
                }
            } else {
                Self::Unknown {
                    type_id: value.type_.0 as i32,
                    raw: std::mem::transmute_copy(&value.__bindgen_anon_1),
                }
            }
        }
    }
}

/// コーデックごとの能力情報
///
/// デコーダーとエンコーダーの対応状況、および対応している場合の能力を表す。
#[derive(Debug, Clone, PartialEq)]
pub struct CodecInfo {
    /// この情報のコーデック種別
    pub codec: VideoCodecType,
    /// デコード能力
    ///
    /// デコードに対応していない場合は `None`。
    pub decoding: Option<DecodingInfo>,
    /// エンコード能力
    ///
    /// エンコードに対応していない場合は `None`。
    pub encoding: Option<EncodingInfo>,
}

/// デコード情報
///
/// `acceleration` / `input` / `output` は、デコーダーの能力情報を取得できなかった場合 `None` になる。
#[derive(Debug, Clone, PartialEq)]
pub struct DecodingInfo {
    /// デコード時のアクセラレーション種別
    pub acceleration: Option<AccelerationType>,
    /// デコーダーの入力の能力情報
    pub input: Option<IoCapabilities>,
    /// デコーダーの出力の能力情報
    pub output: Option<IoCapabilities>,
    /// デコーダーに関する全プロパティ
    ///
    /// 取得できなかった場合は空になる。
    pub properties: Vec<Property>,
}

/// エンコード情報
///
/// `acceleration` / `input` / `output` は、エンコーダーの能力情報を取得できなかった場合 `None` になる。
/// `max_profile` は能力情報を取得できなかった場合と、値が不明な場合に `None` になる。
/// `max_level` は能力情報を取得できなかった場合 `None` になる。
/// `supports_frame_reordering` は対応情報を取得できなかった場合 `None` になる。
#[derive(Debug, Clone, PartialEq)]
pub struct EncodingInfo {
    /// エンコード時のアクセラレーション種別
    pub acceleration: Option<AccelerationType>,
    /// 最大プロファイル
    pub max_profile: Option<VideoProfile>,
    /// 最大レベル
    ///
    /// 値はコーデックごとのレベル enum の値であり、コーデック間で比較できない。
    pub max_level: Option<i64>,
    /// B フレーム (フレームリオーダリング) に対応しているか
    ///
    /// `None` は非対応を意味しない。
    pub supports_frame_reordering: Option<bool>,
    /// エンコーダーの入力の能力情報
    pub input: Option<IoCapabilities>,
    /// エンコーダーの出力の能力情報
    pub output: Option<IoCapabilities>,
    /// エンコーダーに関する全プロパティ
    ///
    /// `max_profile` / `max_level` / `supports_frame_reordering` に使うプロパティも含む。
    /// 全件を取得できなかった場合は空になる。
    pub properties: Vec<Property>,
}

// ---------------------------------------------------------------------------
// 公開関数
// ---------------------------------------------------------------------------

/// このバックエンドで利用可能なコーデックの能力情報を返す
///
/// デコードに対応している場合は `decoding` が `Some`、エンコードに対応している場合は
/// `encoding` が `Some` になる。個々のコーデックが利用できない場合、その方向は `None` になる。
///
/// # Errors
///
/// AMF ランタイムを初期化できない場合にエラーを返す。
pub fn supported_codecs() -> Result<Vec<CodecInfo>, Error> {
    let context = AmfLibrary::instance().create_context()?;
    Ok(VideoCodecType::all()
        .iter()
        .map(|&codec| CodecInfo {
            codec,
            decoding: probe_decoding(&context, codec),
            encoding: probe_encoding(&context, codec),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// 内部実装
// ---------------------------------------------------------------------------

/// エンコーダーのコンポーネント ID を返す
fn encoder_component_id(codec: VideoCodecType) -> &'static str {
    match codec {
        VideoCodecType::H264 => sys::str::AMFVideoEncoderVCE_AVC,
        VideoCodecType::Hevc => sys::str::AMFVideoEncoder_HEVC,
        VideoCodecType::Av1 => sys::str::AMFVideoEncoder_AV1,
    }
}

/// デコーダーのコンポーネント ID を返す
fn decoder_component_id(codec: VideoCodecType) -> &'static str {
    match codec {
        VideoCodecType::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
        VideoCodecType::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
        VideoCodecType::Av1 => sys::str::AMFVideoDecoderHW_AV1,
    }
}

/// 最大プロファイルのプロパティ名を返す
fn profile_property(codec: VideoCodecType) -> &'static str {
    match codec {
        VideoCodecType::H264 => sys::str::AMF_VIDEO_ENCODER_CAP_MAX_PROFILE,
        VideoCodecType::Hevc => sys::str::AMF_VIDEO_ENCODER_HEVC_CAP_MAX_PROFILE,
        VideoCodecType::Av1 => sys::str::AMF_VIDEO_ENCODER_AV1_CAP_MAX_PROFILE,
    }
}

/// 最大レベルのプロパティ名を返す
fn level_property(codec: VideoCodecType) -> &'static str {
    match codec {
        VideoCodecType::H264 => sys::str::AMF_VIDEO_ENCODER_CAP_MAX_LEVEL,
        VideoCodecType::Hevc => sys::str::AMF_VIDEO_ENCODER_HEVC_CAP_MAX_LEVEL,
        VideoCodecType::Av1 => sys::str::AMF_VIDEO_ENCODER_AV1_CAP_MAX_LEVEL,
    }
}

/// B フレーム対応のプロパティ名を返す
///
/// HEVC には B フレーム対応を表す caps プロパティがないため `None` を返す。
fn bframes_property(codec: VideoCodecType) -> Option<&'static str> {
    match codec {
        VideoCodecType::H264 => Some(sys::str::AMF_VIDEO_ENCODER_CAP_BFRAMES),
        VideoCodecType::Hevc => None,
        VideoCodecType::Av1 => Some(sys::str::AMF_VIDEO_ENCODER_AV1_CAP_BFRAMES),
    }
}

/// エンコーダーの能力情報を返す
fn probe_encoding(context: &Context, codec: VideoCodecType) -> Option<EncodingInfo> {
    let component = AmfLibrary::instance()
        .create_component(context, encoder_component_id(codec))
        .ok()?;
    let caps = component.get_caps().ok();

    // Init に渡すサイズは入力 caps の最小値を使う。取得できない場合は、
    // 実機の全コーデックで受け付けられる 640x480 にフォールバックする。
    let (width, height) = caps
        .as_ref()
        .and_then(|caps| caps.input_caps().ok())
        .map(|io_caps| {
            let (min_width, _) = io_caps.width_range();
            let (min_height, _) = io_caps.height_range();
            (min_width, min_height)
        })
        .unwrap_or((640, 480));

    let result = component.init(sys::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, width, height);
    if result != sys::AMF_RESULT::AMF_OK {
        drop(caps);
        let _ = component.terminate();
        return None;
    }

    // GetCaps が Init 前に失敗するランタイムでも能力情報を取得できるよう、
    // Init 後に一度だけ再試行する。
    let caps = match caps {
        Some(caps) => Some(caps),
        None => component.get_caps().ok(),
    };
    let info = encoding_info_from_caps(codec, caps.as_ref());
    drop(caps);
    let _ = component.terminate();
    Some(info)
}

/// デコーダーの能力情報を返す
fn probe_decoding(context: &Context, codec: VideoCodecType) -> Option<DecodingInfo> {
    let component = AmfLibrary::instance()
        .create_component(context, decoder_component_id(codec))
        .ok()?;
    let caps = component.get_caps().ok();

    let result = component.init(sys::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, 0, 0);
    if result != sys::AMF_RESULT::AMF_OK {
        drop(caps);
        let _ = component.terminate();
        return None;
    }

    let caps = match caps {
        Some(caps) => Some(caps),
        None => component.get_caps().ok(),
    };
    let info = decoding_info_from_caps(caps.as_ref());
    drop(caps);
    let _ = component.terminate();
    Some(info)
}

/// エンコーダーの能力情報を構築する
fn encoding_info_from_caps(codec: VideoCodecType, caps: Option<&Caps>) -> EncodingInfo {
    let acceleration = caps.map(|caps| AccelerationType::from_amf(caps.acceleration_type()));
    let max_profile = caps
        .and_then(|caps| caps.get_property_int64(profile_property(codec)))
        .and_then(|value| VideoProfile::from_amf(codec, value));
    let max_level = caps.and_then(|caps| caps.get_property_int64(level_property(codec)));
    let supports_frame_reordering =
        caps.and_then(|caps| bframes_property(codec).and_then(|name| caps.get_property_bool(name)));
    let input = caps
        .and_then(|caps| caps.input_caps().ok())
        .and_then(|io_caps| convert_io_caps(io_caps).ok());
    let output = caps
        .and_then(|caps| caps.output_caps().ok())
        .and_then(|io_caps| convert_io_caps(io_caps).ok());
    let properties = caps
        .and_then(|caps| properties_from_caps(caps).ok())
        .unwrap_or_default();

    EncodingInfo {
        acceleration,
        max_profile,
        max_level,
        supports_frame_reordering,
        input,
        output,
        properties,
    }
}

/// デコーダーの能力情報を構築する
fn decoding_info_from_caps(caps: Option<&Caps>) -> DecodingInfo {
    let acceleration = caps.map(|caps| AccelerationType::from_amf(caps.acceleration_type()));
    let input = caps
        .and_then(|caps| caps.input_caps().ok())
        .and_then(|io_caps| convert_io_caps(io_caps).ok());
    let output = caps
        .and_then(|caps| caps.output_caps().ok())
        .and_then(|io_caps| convert_io_caps(io_caps).ok());
    let properties = caps
        .and_then(|caps| properties_from_caps(caps).ok())
        .unwrap_or_default();

    DecodingInfo {
        acceleration,
        input,
        output,
        properties,
    }
}

/// `AMFIOCaps` を公開型に変換する
fn convert_io_caps(io_caps: RawIoCaps) -> Result<IoCapabilities, Error> {
    let (min_width, max_width) = io_caps.width_range();
    let (min_height, max_height) = io_caps.height_range();
    let vertical_alignment = io_caps.vertical_alignment();
    let formats = io_caps
        .formats()?
        .into_iter()
        .map(|(format, native)| SurfaceFormatCapability {
            format: SurfaceFormat::from_amf(format),
            native,
        })
        .collect();
    let memory_types = io_caps
        .memory_types()?
        .into_iter()
        .map(|(memory_type, native)| MemoryTypeCapability {
            memory_type: MemoryType::from_amf(memory_type),
            native,
        })
        .collect();
    let interlaced = io_caps.interlaced();
    Ok(IoCapabilities {
        min_width,
        max_width,
        min_height,
        max_height,
        vertical_alignment,
        formats,
        memory_types,
        interlaced,
    })
}

/// `AMFCaps` の全プロパティを取得する
fn properties_from_caps(caps: &Caps) -> Result<Vec<Property>, Error> {
    let count = caps.property_count();
    let mut properties = Vec::new();
    for index in 0..count {
        let (name, value) = caps.property_at(index)?;
        properties.push(Property {
            name,
            value: Value::from_variant(value.as_ref()),
        });
    }
    Ok(properties)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の Context を作成する
    fn create_context() -> Context {
        AmfLibrary::instance()
            .create_context()
            .expect("Context の作成に失敗")
    }

    /// 入力 caps の最小サイズでエンコーダーを初期化する
    fn init_encoder(component: &crate::amf::Component, caps: &Caps) -> sys::AMF_RESULT {
        let (width, height) = caps
            .input_caps()
            .ok()
            .map(|io_caps| {
                let (min_width, _) = io_caps.width_range();
                let (min_height, _) = io_caps.height_range();
                if min_width > 0 && min_height > 0 {
                    (min_width, min_height)
                } else {
                    (640, 480)
                }
            })
            .unwrap_or((640, 480));
        component.init(sys::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, width, height)
    }

    /// 公開 IoCapabilities が AMF の値と一致することを確認する
    fn assert_io_caps_matches(
        label: &str,
        actual: Option<&IoCapabilities>,
        raw: Result<RawIoCaps, Error>,
    ) {
        let actual = actual.unwrap_or_else(|| panic!("{label} が None になっている"));
        let raw = raw.unwrap_or_else(|e| panic!("{label} の取得に失敗した: {e}"));
        let (min_width, max_width) = raw.width_range();
        let (min_height, max_height) = raw.height_range();
        assert_eq!(
            (actual.min_width, actual.max_width),
            (min_width, max_width),
            "{label} の幅の範囲が一致しない"
        );
        assert_eq!(
            (actual.min_height, actual.max_height),
            (min_height, max_height),
            "{label} の高さの範囲が一致しない"
        );
        assert_eq!(
            actual.vertical_alignment,
            raw.vertical_alignment(),
            "{label} の垂直アライメントが一致しない"
        );
        assert_eq!(
            actual.formats.len(),
            raw.formats().expect("formats の取得に失敗").len(),
            "{label} のフォーマット数が一致しない"
        );
        assert_eq!(
            actual.memory_types.len(),
            raw.memory_types().expect("memory_types の取得に失敗").len(),
            "{label} のメモリタイプ数が一致しない"
        );
        assert_eq!(
            actual.interlaced,
            raw.interlaced(),
            "{label} のインターレース対応が一致しない"
        );
    }

    /// エンコーダーの能力情報が AMFCaps の値と一致することを確認する
    #[test]
    fn test_encoding_info_matches_caps() {
        let context = create_context();
        for &codec in VideoCodecType::all() {
            let component = AmfLibrary::instance()
                .create_component(&context, encoder_component_id(codec))
                .expect("エンコーダーの作成に失敗");
            let caps = component.get_caps().expect("GetCaps に失敗");
            assert_eq!(
                init_encoder(&component, &caps),
                sys::AMF_RESULT::AMF_OK,
                "{codec:?} のエンコーダー初期化に失敗"
            );

            let info = encoding_info_from_caps(codec, Some(&caps));
            let expected_profile = caps
                .get_property_int64(profile_property(codec))
                .and_then(|value| VideoProfile::from_amf(codec, value));
            assert_eq!(
                info.max_profile, expected_profile,
                "{codec:?} の max_profile が一致しない"
            );
            assert_eq!(
                info.max_level,
                caps.get_property_int64(level_property(codec)),
                "{codec:?} の max_level が一致しない"
            );
            assert_eq!(
                info.supports_frame_reordering,
                bframes_property(codec).and_then(|name| caps.get_property_bool(name)),
                "{codec:?} の supports_frame_reordering が一致しない"
            );
            assert_eq!(
                info.properties.len(),
                caps.property_count(),
                "{codec:?} のプロパティ数が一致しない"
            );

            // フィールドに写像したプロパティも properties に含まれることを確認する
            if let Some(value) = caps.get_property_int64(profile_property(codec)) {
                let property = info
                    .properties
                    .iter()
                    .find(|property| property.name == profile_property(codec))
                    .unwrap_or_else(|| panic!("{codec:?} のプロパティが見つからない"));
                assert_eq!(
                    property.value,
                    Value::Int64(value),
                    "{codec:?} のプロパティ値が一致しない"
                );
            }

            assert_io_caps_matches(
                &format!("{codec:?} の入力"),
                info.input.as_ref(),
                caps.input_caps(),
            );
            assert_io_caps_matches(
                &format!("{codec:?} の出力"),
                info.output.as_ref(),
                caps.output_caps(),
            );

            drop(caps);
            let _ = component.terminate();
        }
    }

    /// デコーダーの能力情報が AMFCaps の値と一致することを確認する
    #[test]
    fn test_decoding_info_matches_caps() {
        let context = create_context();
        for &codec in VideoCodecType::all() {
            let component = AmfLibrary::instance()
                .create_component(&context, decoder_component_id(codec))
                .expect("デコーダーの作成に失敗");
            let caps = component.get_caps().expect("GetCaps に失敗");
            assert_eq!(
                component.init(sys::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, 0, 0),
                sys::AMF_RESULT::AMF_OK,
                "{codec:?} のデコーダー初期化に失敗"
            );

            let info = decoding_info_from_caps(Some(&caps));
            assert_eq!(
                info.acceleration,
                Some(AccelerationType::from_amf(caps.acceleration_type())),
                "{codec:?} の acceleration が一致しない"
            );
            assert_eq!(
                info.properties.len(),
                caps.property_count(),
                "{codec:?} のプロパティ数が一致しない"
            );
            assert_io_caps_matches(
                &format!("{codec:?} の入力"),
                info.input.as_ref(),
                caps.input_caps(),
            );
            assert_io_caps_matches(
                &format!("{codec:?} の出力"),
                info.output.as_ref(),
                caps.output_caps(),
            );

            drop(caps);
            let _ = component.terminate();
        }
    }
}
