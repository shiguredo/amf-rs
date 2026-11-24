//! エンコーダー・コーデック情報・AMF オブジェクトで共有する型

use crate::sys;

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
    /// Semi-Planar YUV 4:2:0 10bit (16bit 格納、上位 10bit)
    P010,
    /// Semi-Planar YUV 4:2:0 12bit (16bit 格納、上位 12bit)
    P012,
    /// Semi-Planar YUV 4:2:0 16bit (16bit 格納、全ビット)
    P016,
    /// Packed YUV 4:2:2 10bit (16bit ワードの上位 10bit)
    Y210,
    /// Packed YUV 4:4:4 8bit
    Ayuv,
    /// Packed YUV 4:4:4 10bit (32bit ワード、下位から U (10bit) / Y (10bit) / V (10bit) / A (2bit))
    Y410,
    /// Packed YUV 4:4:4 16bit (16bit ワード、全ビット)
    Y416,
}

impl FrameFormat {
    /// `AMF_SURFACE_FORMAT` の値に変換する
    pub(crate) fn to_amf(self) -> sys::AMF_SURFACE_FORMAT {
        match self {
            Self::Nv12 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12,
            Self::Yv12 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YV12,
            Self::I420 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YUV420P,
            Self::Bgra => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_BGRA,
            Self::Argb => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_ARGB,
            Self::Rgba => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_RGBA,
            Self::Yuy2 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YUY2,
            Self::Uyvy => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_UYVY,
            Self::P010 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P010,
            Self::P012 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P012,
            Self::P016 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P016,
            Self::Y210 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y210,
            Self::Ayuv => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_AYUV,
            Self::Y410 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y410,
            Self::Y416 => sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y416,
        }
    }
}

/// AMF の surface format
///
/// AMF が報告する値と、AMF に渡す値の両方に使う。`FrameFormat` に写像できる値は
/// [`SurfaceFormat::Known`] に、写像できない値は [`SurfaceFormat::Unknown`] に
/// AMF の生値を保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceFormat {
    /// `FrameFormat` に写像できる形式
    Known(FrameFormat),
    /// `FrameFormat` に写像できない形式
    Unknown(u32),
}

impl SurfaceFormat {
    /// `AMF_SURFACE_FORMAT` の値から変換する
    pub(crate) fn from_amf(format: sys::AMF_SURFACE_FORMAT) -> Self {
        match format {
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12 => Self::Known(FrameFormat::Nv12),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YV12 => Self::Known(FrameFormat::Yv12),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YUV420P => Self::Known(FrameFormat::I420),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_BGRA => Self::Known(FrameFormat::Bgra),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_ARGB => Self::Known(FrameFormat::Argb),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_RGBA => Self::Known(FrameFormat::Rgba),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_YUY2 => Self::Known(FrameFormat::Yuy2),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_UYVY => Self::Known(FrameFormat::Uyvy),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P010 => Self::Known(FrameFormat::P010),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P012 => Self::Known(FrameFormat::P012),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_P016 => Self::Known(FrameFormat::P016),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y210 => Self::Known(FrameFormat::Y210),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_AYUV => Self::Known(FrameFormat::Ayuv),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y410 => Self::Known(FrameFormat::Y410),
            sys::AMF_SURFACE_FORMAT::AMF_SURFACE_Y416 => Self::Known(FrameFormat::Y416),
            other => Self::Unknown(other.0),
        }
    }

    /// `AMF_SURFACE_FORMAT` の値に変換する
    pub(crate) fn to_amf(self) -> sys::AMF_SURFACE_FORMAT {
        match self {
            Self::Known(format) => format.to_amf(),
            Self::Unknown(raw) => sys::AMF_SURFACE_FORMAT(raw),
        }
    }
}

impl From<FrameFormat> for SurfaceFormat {
    fn from(format: FrameFormat) -> Self {
        Self::Known(format)
    }
}

/// AMF のメモリ種別
///
/// AMF が定義する memory type のうち、本クレートが名前付きで扱う種別を表す。
/// 名前付きの種別に該当しない値は [`MemoryType::Unknown`] に AMF の生値を保持する。
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
    /// 本クレートが名前付きで扱わない AMF の memory type
    ///
    /// AMF の `AMF_MEMORY_TYPE` の値そのものを保持する。
    Unknown(u32),
}

impl MemoryType {
    /// `AMF_MEMORY_TYPE` の値から変換する
    pub(crate) fn from_amf(memory_type: sys::AMF_MEMORY_TYPE) -> Self {
        match memory_type {
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_HOST => Self::Host,
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_OPENCL => Self::OpenCl,
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_OPENGL => Self::OpenGl,
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_XV => Self::Xv,
            sys::AMF_MEMORY_TYPE::AMF_MEMORY_VULKAN => Self::Vulkan,
            other => Self::Unknown(other.0),
        }
    }

    /// `AMF_MEMORY_TYPE` の値に変換する
    pub(crate) fn to_amf(self) -> sys::AMF_MEMORY_TYPE {
        match self {
            Self::Host => sys::AMF_MEMORY_TYPE::AMF_MEMORY_HOST,
            Self::OpenCl => sys::AMF_MEMORY_TYPE::AMF_MEMORY_OPENCL,
            Self::OpenGl => sys::AMF_MEMORY_TYPE::AMF_MEMORY_OPENGL,
            Self::Xv => sys::AMF_MEMORY_TYPE::AMF_MEMORY_XV,
            Self::Vulkan => sys::AMF_MEMORY_TYPE::AMF_MEMORY_VULKAN,
            Self::Unknown(raw) => sys::AMF_MEMORY_TYPE(raw),
        }
    }
}

/// AMF のデータ種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// 汎用バッファ
    Buffer,
    /// サーフェス
    Surface,
    /// オーディオバッファ
    AudioBuffer,
    /// ユーザーデータ
    User,
    /// 本クレートが名前付きで扱わない AMF のデータ種別
    ///
    /// AMF の `AMF_DATA_TYPE` の値そのものを保持する。
    Unknown(u32),
}

impl DataType {
    /// `AMF_DATA_TYPE` の値から変換する
    pub(crate) fn from_amf(data_type: sys::AMF_DATA_TYPE) -> Self {
        match data_type {
            sys::AMF_DATA_TYPE::AMF_DATA_BUFFER => Self::Buffer,
            sys::AMF_DATA_TYPE::AMF_DATA_SURFACE => Self::Surface,
            sys::AMF_DATA_TYPE::AMF_DATA_AUDIO_BUFFER => Self::AudioBuffer,
            sys::AMF_DATA_TYPE::AMF_DATA_USER => Self::User,
            other => Self::Unknown(other.0),
        }
    }
}

/// AMF のプレーン種別
///
/// [`crate::amf::Surface::get_plane`] に指定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneType {
    /// パック形式のプレーン (BGRA / YUY2 など)
    Packed,
    /// Y プレーン
    Y,
    /// UV をインターリーブしたプレーン
    Uv,
    /// U プレーン
    U,
    /// V プレーン
    V,
}

impl PlaneType {
    /// `AMF_PLANE_TYPE` の値に変換する
    pub(crate) fn to_amf(self) -> sys::AMF_PLANE_TYPE {
        match self {
            Self::Packed => sys::AMF_PLANE_TYPE::AMF_PLANE_PACKED,
            Self::Y => sys::AMF_PLANE_TYPE::AMF_PLANE_Y,
            Self::Uv => sys::AMF_PLANE_TYPE::AMF_PLANE_UV,
            Self::U => sys::AMF_PLANE_TYPE::AMF_PLANE_U,
            Self::V => sys::AMF_PLANE_TYPE::AMF_PLANE_V,
        }
    }
}

/// H.264 プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Profile {
    /// Baseline プロファイル
    Baseline,
    /// Main プロファイル
    Main,
    /// High プロファイル
    High,
    /// Constrained Baseline プロファイル
    ConstrainedBaseline,
    /// Constrained High プロファイル
    ConstrainedHigh,
}

/// HEVC プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcProfile {
    /// Main プロファイル
    Main,
    /// Main 10 プロファイル
    Main10,
}

/// AV1 プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Av1Profile {
    /// Main プロファイル
    Main,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AMF_DATA_TYPE の各値が DataType に変換されることを確認する
    #[test]
    fn test_data_type_from_amf() {
        assert_eq!(
            DataType::from_amf(sys::AMF_DATA_TYPE::AMF_DATA_BUFFER),
            DataType::Buffer
        );
        assert_eq!(
            DataType::from_amf(sys::AMF_DATA_TYPE::AMF_DATA_SURFACE),
            DataType::Surface
        );
        assert_eq!(
            DataType::from_amf(sys::AMF_DATA_TYPE::AMF_DATA_AUDIO_BUFFER),
            DataType::AudioBuffer
        );
        assert_eq!(
            DataType::from_amf(sys::AMF_DATA_TYPE::AMF_DATA_USER),
            DataType::User
        );
        assert_eq!(
            DataType::from_amf(sys::AMF_DATA_TYPE(1001)),
            DataType::Unknown(1001)
        );
    }
}
