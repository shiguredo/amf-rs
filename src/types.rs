//! エンコーダーとコーデック情報で共有する型

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
