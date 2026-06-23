use shiguredo_amf::{VideoCodecType, supported_codecs};

/// supported_codecs() がパニックせずに結果を返すことを確認する
///
/// AMF ランタイムがない環境でも安全に動作する。
#[test]
fn test_supported_codecs_returns_all_codecs() {
    let codecs = supported_codecs();
    assert_eq!(codecs.len(), 3);
    assert_eq!(codecs[0].codec, VideoCodecType::H264);
    assert_eq!(codecs[1].codec, VideoCodecType::Hevc);
    assert_eq!(codecs[2].codec, VideoCodecType::Av1);
}

/// デコードとエンコードの supported フラグの整合性を確認する
///
/// hardware_accelerated は常に supported と同値であること。
#[test]
fn test_supported_codecs_hw_accel_consistency() {
    let codecs = supported_codecs();
    for info in &codecs {
        assert_eq!(
            info.decoding.supported, info.decoding.hardware_accelerated,
            "{:?}: decoding の supported と hardware_accelerated が一致しない",
            info.codec
        );
        assert_eq!(
            info.encoding.supported, info.encoding.hardware_accelerated,
            "{:?}: encoding の supported と hardware_accelerated が一致しない",
            info.codec
        );
    }
}
