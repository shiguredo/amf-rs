//! コーデック情報の照会

use std::ffi::CStr;

use crate::AmfLibrary;
use crate::amf::{Caps, Context, IoCaps};
use crate::error::Error;
use crate::sys;
use crate::types::{Av1Profile, H264Profile, HevcProfile, MemoryType, SurfaceFormat};

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
    /// VP9 コーデック
    Vp9,
}

impl VideoCodecType {
    /// すべてのコーデック種別を返す
    fn all() -> &'static [Self] {
        &[Self::H264, Self::Hevc, Self::Av1, Self::Vp9]
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
    /// AMF の生値
    ///
    /// 値の意味はコーデックごとに異なり、コーデック間で比較できない。
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
            // VP9 は AMF のエンコーダーを持たないため、プロファイル値を扱わない。
            VideoCodecType::Vp9 => None,
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
    ///
    /// H.264 の値 0 はプロファイル不明として `None` になる。HEVC / AV1 の値 0 は `Unknown(0)`。
    pub max_profile: Option<VideoProfile>,
    /// 最大レベル
    ///
    /// 値はコーデックごとのレベル enum の値であり、コーデック間で比較できない。
    pub max_level: Option<i64>,
    /// B フレーム (フレームリオーダリング) に対応しているか
    ///
    /// AMF が対応情報を返さない場合は `None`。`Some(false)` は非対応を表す。
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
/// AMF ランタイムを初期化できない場合、または Vulkan デバイスの初期化に失敗した
/// 場合にエラーを返す。
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

/// エンコーダーを提供するコーデックのコンポーネント ID を返す
///
/// AMF がエンコーダーを提供しないコーデックの場合は `None` を返す。
fn encoder_component_id(codec: VideoCodecType) -> Option<&'static str> {
    match codec {
        VideoCodecType::H264 => Some(sys::str::AMFVideoEncoderVCE_AVC),
        VideoCodecType::Hevc => Some(sys::str::AMFVideoEncoder_HEVC),
        VideoCodecType::Av1 => Some(sys::str::AMFVideoEncoder_AV1),
        VideoCodecType::Vp9 => None,
    }
}

/// デコーダーのコンポーネント ID を返す
fn decoder_component_id(codec: VideoCodecType) -> &'static str {
    match codec {
        VideoCodecType::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
        VideoCodecType::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
        VideoCodecType::Av1 => sys::str::AMFVideoDecoderHW_AV1,
        VideoCodecType::Vp9 => sys::str::AMFVideoDecoderHW_VP9,
    }
}

/// 最大プロファイルのプロパティ名を返す
fn max_profile_property(codec: VideoCodecType) -> Option<&'static str> {
    match codec {
        VideoCodecType::H264 => Some(sys::str::AMF_VIDEO_ENCODER_CAP_MAX_PROFILE),
        VideoCodecType::Hevc => Some(sys::str::AMF_VIDEO_ENCODER_HEVC_CAP_MAX_PROFILE),
        VideoCodecType::Av1 => Some(sys::str::AMF_VIDEO_ENCODER_AV1_CAP_MAX_PROFILE),
        VideoCodecType::Vp9 => None,
    }
}

/// 最大レベルのプロパティ名を返す
fn max_level_property(codec: VideoCodecType) -> Option<&'static str> {
    match codec {
        VideoCodecType::H264 => Some(sys::str::AMF_VIDEO_ENCODER_CAP_MAX_LEVEL),
        VideoCodecType::Hevc => Some(sys::str::AMF_VIDEO_ENCODER_HEVC_CAP_MAX_LEVEL),
        VideoCodecType::Av1 => Some(sys::str::AMF_VIDEO_ENCODER_AV1_CAP_MAX_LEVEL),
        VideoCodecType::Vp9 => None,
    }
}

/// B フレーム対応のプロパティ名を返す
///
/// HEVC には B フレーム対応を表す caps プロパティがないため `None` を返す。
fn supports_frame_reordering_property(codec: VideoCodecType) -> Option<&'static str> {
    match codec {
        VideoCodecType::H264 => Some(sys::str::AMF_VIDEO_ENCODER_CAP_BFRAMES),
        VideoCodecType::Hevc => None,
        VideoCodecType::Av1 => Some(sys::str::AMF_VIDEO_ENCODER_AV1_CAP_BFRAMES),
        VideoCodecType::Vp9 => None,
    }
}

/// エンコーダーの能力情報を返す
///
/// AMF がエンコーダーを提供しないコーデックの場合は `None` を返す。
fn probe_encoding(context: &Context, codec: VideoCodecType) -> Option<EncodingInfo> {
    let component_id = encoder_component_id(codec)?;
    let component = AmfLibrary::instance()
        .create_component(context, component_id)
        .ok()?;
    let caps = component.get_caps().ok();
    Some(build_encoding_info(codec, caps.as_ref()))
}

/// デコーダーの能力情報を返す
fn probe_decoding(context: &Context, codec: VideoCodecType) -> Option<DecodingInfo> {
    let component = AmfLibrary::instance()
        .create_component(context, decoder_component_id(codec))
        .ok()?;
    let caps = component.get_caps().ok();
    Some(build_decoding_info(caps.as_ref()))
}

/// エンコーダーの能力情報を構築する
fn build_encoding_info(codec: VideoCodecType, caps: Option<&Caps>) -> EncodingInfo {
    let acceleration = caps.map(|caps| AccelerationType::from_amf(caps.get_acceleration_type()));
    let max_profile = max_profile_property(codec)
        .and_then(|name| caps.and_then(|caps| caps.get_property(name)))
        .and_then(|value| value.as_int64())
        .and_then(|value| VideoProfile::from_amf(codec, value));
    let max_level = max_level_property(codec)
        .and_then(|name| caps.and_then(|caps| caps.get_property(name)))
        .and_then(|value| value.as_int64());
    let supports_frame_reordering = caps.and_then(|caps| {
        supports_frame_reordering_property(codec)
            .and_then(|name| caps.get_property(name))
            .and_then(|value| value.as_bool())
    });
    let input = caps
        .and_then(|caps| caps.get_input_caps().ok())
        .and_then(|io_caps| convert_io_capabilities(io_caps).ok());
    let output = caps
        .and_then(|caps| caps.get_output_caps().ok())
        .and_then(|io_caps| convert_io_capabilities(io_caps).ok());
    let properties = caps
        .and_then(|caps| collect_properties(caps).ok())
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
fn build_decoding_info(caps: Option<&Caps>) -> DecodingInfo {
    let acceleration = caps.map(|caps| AccelerationType::from_amf(caps.get_acceleration_type()));
    let input = caps
        .and_then(|caps| caps.get_input_caps().ok())
        .and_then(|io_caps| convert_io_capabilities(io_caps).ok());
    let output = caps
        .and_then(|caps| caps.get_output_caps().ok())
        .and_then(|io_caps| convert_io_capabilities(io_caps).ok());
    let properties = caps
        .and_then(|caps| collect_properties(caps).ok())
        .unwrap_or_default();

    DecodingInfo {
        acceleration,
        input,
        output,
        properties,
    }
}

/// `AMFIOCaps` を公開型に変換する
fn convert_io_capabilities(io_caps: IoCaps) -> Result<IoCapabilities, Error> {
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
fn collect_properties(caps: &Caps) -> Result<Vec<Property>, Error> {
    let count = caps.get_property_count();
    let mut properties = Vec::new();
    for index in 0..count {
        let (name, value) = caps.get_property_at(index)?;
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

    use crate::types::FrameFormat;

    /// テスト用の Context を作成する
    fn create_context() -> Context {
        AmfLibrary::instance()
            .create_context()
            .expect("Context の作成に失敗")
    }

    /// テスト側で AMF の surface format を期待値に変換する
    fn expected_surface_format(format: sys::AMF_SURFACE_FORMAT) -> SurfaceFormat {
        match format {
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12 => SurfaceFormat::Known(FrameFormat::Nv12),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YV12 => SurfaceFormat::Known(FrameFormat::Yv12),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YUV420P => SurfaceFormat::Known(FrameFormat::I420),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_BGRA => SurfaceFormat::Known(FrameFormat::Bgra),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_ARGB => SurfaceFormat::Known(FrameFormat::Argb),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_RGBA => SurfaceFormat::Known(FrameFormat::Rgba),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YUY2 => SurfaceFormat::Known(FrameFormat::Yuy2),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_UYVY => SurfaceFormat::Known(FrameFormat::Uyvy),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P010 => SurfaceFormat::Known(FrameFormat::P010),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P012 => SurfaceFormat::Known(FrameFormat::P012),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P016 => SurfaceFormat::Known(FrameFormat::P016),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y210 => SurfaceFormat::Known(FrameFormat::Y210),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_AYUV => SurfaceFormat::Known(FrameFormat::Ayuv),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y410 => SurfaceFormat::Known(FrameFormat::Y410),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y416 => SurfaceFormat::Known(FrameFormat::Y416),
            other => SurfaceFormat::Unknown(other.0),
        }
    }

    /// テスト側で AMF の memory type を期待値に変換する
    fn expected_memory_type(memory_type: sys::AMF_MEMORY_TYPE) -> MemoryType {
        match memory_type {
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_HOST => MemoryType::Host,
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_OPENCL => MemoryType::OpenCl,
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_OPENGL => MemoryType::OpenGl,
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_XV => MemoryType::Xv,
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_VULKAN => MemoryType::Vulkan,
            other => MemoryType::Unknown(other.0),
        }
    }

    /// 公開 IoCapabilities が AMF の値と一致することを確認する
    fn assert_io_capabilities_match(
        label: &str,
        actual: Option<&IoCapabilities>,
        raw: Result<IoCaps, Error>,
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

        let raw_formats = raw.formats().expect("formats の取得に失敗");
        assert_eq!(
            actual.formats.len(),
            raw_formats.len(),
            "{label} のフォーマット数が一致しない"
        );
        for (index, (actual, (raw_format, raw_native))) in
            actual.formats.iter().zip(raw_formats).enumerate()
        {
            assert_eq!(
                actual.format,
                expected_surface_format(raw_format),
                "{label} の {index} 番目の surface format が一致しない"
            );
            assert_eq!(
                actual.native, raw_native,
                "{label} の {index} 番目の native フラグが一致しない"
            );
        }

        let raw_memory_types = raw.memory_types().expect("memory_types の取得に失敗");
        assert_eq!(
            actual.memory_types.len(),
            raw_memory_types.len(),
            "{label} のメモリタイプ数が一致しない"
        );
        for (index, (actual, (raw_memory_type, raw_native))) in
            actual.memory_types.iter().zip(raw_memory_types).enumerate()
        {
            assert_eq!(
                actual.memory_type,
                expected_memory_type(raw_memory_type),
                "{label} の {index} 番目の memory type が一致しない"
            );
            assert_eq!(
                actual.native, raw_native,
                "{label} の {index} 番目の native フラグが一致しない"
            );
        }

        assert_eq!(
            actual.interlaced,
            raw.interlaced(),
            "{label} のインターレース対応が一致しない"
        );
    }

    /// テスト対象の GPU で報告されるエンコーダー能力の期待値を返す
    ///
    /// VP9 は AMF のエンコーダーを持たないため `None` を返す。
    fn expected_encoding(codec: VideoCodecType) -> Option<(VideoProfile, i64, i64, Option<bool>)> {
        match codec {
            VideoCodecType::H264 => {
                Some((VideoProfile::H264(H264Profile::High), 100, 52, Some(true)))
            }
            VideoCodecType::Hevc => Some((VideoProfile::Hevc(HevcProfile::Main10), 2, 186, None)),
            VideoCodecType::Av1 => Some((VideoProfile::Av1(Av1Profile::Main), 1, 16, Some(false))),
            VideoCodecType::Vp9 => None,
        }
    }

    /// エンコーダーの能力情報が AMFCaps の値と一致することを確認する
    #[test]
    fn test_encoding_info_matches_caps() {
        let context = create_context();
        for &codec in VideoCodecType::all() {
            // VP9 は AMF のエンコーダーを持たないため、エンコーダー能力を検証しない。
            let Some(component_id) = encoder_component_id(codec) else {
                continue;
            };
            let (profile, profile_value, level, frame_reordering) =
                expected_encoding(codec).expect("エンコーダーを持つコーデックの期待値がない");
            let component = AmfLibrary::instance()
                .create_component(&context, component_id)
                .expect("エンコーダーの作成に失敗");
            let caps = component.get_caps().expect("GetCaps に失敗");
            let profile_property_name =
                max_profile_property(codec).expect("最大プロファイルのプロパティ名がない");
            let level_property_name =
                max_level_property(codec).expect("最大レベルのプロパティ名がない");

            assert_eq!(
                caps.get_property(profile_property_name)
                    .and_then(|value| value.as_int64()),
                Some(profile_value),
                "{codec:?} の最大プロファイルプロパティが期待値と一致しない"
            );
            assert_eq!(
                caps.get_property(level_property_name)
                    .and_then(|value| value.as_int64()),
                Some(level),
                "{codec:?} の最大レベルプロパティが期待値と一致しない"
            );
            if let Some(name) = supports_frame_reordering_property(codec) {
                assert_eq!(
                    caps.get_property(name).and_then(|value| value.as_bool()),
                    frame_reordering,
                    "{codec:?} の B フレームプロパティが期待値と一致しない"
                );
            }

            let info = build_encoding_info(codec, Some(&caps));
            assert_eq!(
                info.acceleration,
                Some(AccelerationType::Hardware),
                "{codec:?} の acceleration が期待値と一致しない"
            );
            assert_eq!(
                info.max_profile,
                Some(profile),
                "{codec:?} の max_profile が期待値と一致しない"
            );
            assert_eq!(
                info.max_level,
                Some(level),
                "{codec:?} の max_level が期待値と一致しない"
            );
            assert_eq!(
                info.supports_frame_reordering, frame_reordering,
                "{codec:?} の supports_frame_reordering が期待値と一致しない"
            );
            assert_eq!(
                info.properties.len(),
                caps.get_property_count(),
                "{codec:?} のプロパティ数が一致しない"
            );

            for (index, property) in info.properties.iter().enumerate() {
                let (name, value) = caps.get_property_at(index).expect("プロパティの取得に失敗");
                assert_eq!(
                    property.name, name,
                    "{codec:?} の {index} 番目のプロパティ名が一致しない"
                );
                assert_eq!(
                    property.value,
                    Value::from_variant(value.as_ref()),
                    "{codec:?} の {index} 番目のプロパティ値が一致しない"
                );
            }

            let profile_property = info
                .properties
                .iter()
                .find(|property| property.name == profile_property_name)
                .unwrap_or_else(|| panic!("{codec:?} の最大プロファイルプロパティが見つからない"));
            assert_eq!(
                profile_property.value,
                Value::Int64(profile_value),
                "{codec:?} の最大プロファイルプロパティ値が一致しない"
            );

            let level_property = info
                .properties
                .iter()
                .find(|property| property.name == level_property_name)
                .unwrap_or_else(|| panic!("{codec:?} の最大レベルプロパティが見つからない"));
            assert_eq!(
                level_property.value,
                Value::Int64(level),
                "{codec:?} の最大レベルプロパティ値が一致しない"
            );

            if let Some(name) = supports_frame_reordering_property(codec) {
                let property = info
                    .properties
                    .iter()
                    .find(|property| property.name == name)
                    .unwrap_or_else(|| panic!("{codec:?} の B フレームプロパティが見つからない"));
                assert_eq!(
                    property.value,
                    Value::Bool(frame_reordering.expect("B フレーム対応の期待値がない")),
                    "{codec:?} の B フレームプロパティ値が一致しない"
                );
            }

            assert_io_capabilities_match(
                &format!("{codec:?} の入力"),
                info.input.as_ref(),
                caps.get_input_caps(),
            );
            assert_io_capabilities_match(
                &format!("{codec:?} の出力"),
                info.output.as_ref(),
                caps.get_output_caps(),
            );
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

            let info = build_decoding_info(Some(&caps));
            assert_eq!(
                info.acceleration,
                Some(AccelerationType::Hardware),
                "{codec:?} の acceleration が期待値と一致しない"
            );
            assert_eq!(
                info.properties.len(),
                caps.get_property_count(),
                "{codec:?} のプロパティ数が一致しない"
            );
            for (index, property) in info.properties.iter().enumerate() {
                let (name, value) = caps.get_property_at(index).expect("プロパティの取得に失敗");
                assert_eq!(
                    property.name, name,
                    "{codec:?} の {index} 番目のプロパティ名が一致しない"
                );
                assert_eq!(
                    property.value,
                    Value::from_variant(value.as_ref()),
                    "{codec:?} の {index} 番目のプロパティ値が一致しない"
                );
            }
            assert_io_capabilities_match(
                &format!("{codec:?} の入力"),
                info.input.as_ref(),
                caps.get_input_caps(),
            );
            assert_io_capabilities_match(
                &format!("{codec:?} の出力"),
                info.output.as_ref(),
                caps.get_output_caps(),
            );
        }
    }
    /// `probe_encoding` / `probe_decoding` が能力情報を取得できることを確認する
    ///
    /// VP9 は AMF のエンコーダーを持たないため、エンコーダー能力が `None` になることを確認する。
    #[test]
    fn test_probe_returns_capabilities() {
        let context = create_context();
        for &codec in VideoCodecType::all() {
            match probe_encoding(&context, codec) {
                Some(encoding) => {
                    assert_eq!(
                        encoding.acceleration,
                        Some(AccelerationType::Hardware),
                        "{codec:?} のエンコーダー acceleration が期待値と一致しない"
                    );
                    assert!(
                        encoding.input.is_some(),
                        "{codec:?} のエンコーダー入力能力がない"
                    );
                    assert!(
                        encoding.output.is_some(),
                        "{codec:?} のエンコーダー出力能力がない"
                    );
                    assert!(
                        !encoding.properties.is_empty(),
                        "{codec:?} のエンコーダープロパティがない"
                    );
                }
                None => assert_eq!(
                    codec,
                    VideoCodecType::Vp9,
                    "{codec:?} のエンコーダー能力を取得できない"
                ),
            }

            let decoding = probe_decoding(&context, codec)
                .unwrap_or_else(|| panic!("{codec:?} のデコーダー能力を取得できない"));
            assert_eq!(
                decoding.acceleration,
                Some(AccelerationType::Hardware),
                "{codec:?} のデコーダー acceleration が期待値と一致しない"
            );
            assert!(
                decoding.input.is_some(),
                "{codec:?} のデコーダー入力能力がない"
            );
            assert!(
                decoding.output.is_some(),
                "{codec:?} のデコーダー出力能力がない"
            );
            assert!(
                !decoding.properties.is_empty(),
                "{codec:?} のデコーダープロパティがない"
            );
        }
    }

    /// 能力情報がない場合のエンコーダー情報の契約を確認する
    #[test]
    fn test_build_encoding_info_none() {
        for &codec in VideoCodecType::all() {
            let info = build_encoding_info(codec, None);
            assert_eq!(
                info.acceleration, None,
                "{codec:?} の acceleration が None ではない"
            );
            assert_eq!(
                info.max_profile, None,
                "{codec:?} の max_profile が None ではない"
            );
            assert_eq!(
                info.max_level, None,
                "{codec:?} の max_level が None ではない"
            );
            assert_eq!(
                info.supports_frame_reordering, None,
                "{codec:?} の supports_frame_reordering が None ではない"
            );
            assert!(info.input.is_none(), "{codec:?} の入力能力が None ではない");
            assert!(
                info.output.is_none(),
                "{codec:?} の出力能力が None ではない"
            );
            assert!(
                info.properties.is_empty(),
                "{codec:?} のプロパティが空ではない"
            );
        }
    }

    /// 能力情報がない場合のデコーダー情報の契約を確認する
    #[test]
    fn test_build_decoding_info_none() {
        for &codec in VideoCodecType::all() {
            let info = build_decoding_info(None);
            assert_eq!(
                info.acceleration, None,
                "{codec:?} の acceleration が None ではない"
            );
            assert!(info.input.is_none(), "{codec:?} の入力能力が None ではない");
            assert!(
                info.output.is_none(),
                "{codec:?} の出力能力が None ではない"
            );
            assert!(
                info.properties.is_empty(),
                "{codec:?} のプロパティが空ではない"
            );
        }
    }
}
