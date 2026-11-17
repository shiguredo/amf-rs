use shiguredo_amf::{VideoCodecType, VideoProfile, supported_codecs};

/// `supported_codecs` が全コーデックを返すことを確認する
#[test]
fn test_supported_codecs_returns_all_codecs() {
    let codecs = supported_codecs().expect("supported_codecs が失敗した");
    assert_eq!(codecs.len(), 3, "コーデック数が 3 ではない: {codecs:?}");
    for codec in [
        VideoCodecType::H264,
        VideoCodecType::Hevc,
        VideoCodecType::Av1,
    ] {
        assert!(
            codecs.iter().any(|info| info.codec == codec),
            "{codec:?} が含まれない: {codecs:?}"
        );
    }
}

/// `max_profile` の variant がコーデックと一致することを確認する
#[test]
fn test_encoding_profile_matches_codec() {
    for info in supported_codecs().expect("supported_codecs が失敗した") {
        let Some(encoding) = info.encoding else {
            continue;
        };
        let Some(profile) = encoding.max_profile else {
            continue;
        };
        match (info.codec, profile) {
            (VideoCodecType::H264, VideoProfile::H264(_)) => {}
            (VideoCodecType::Hevc, VideoProfile::Hevc(_)) => {}
            (VideoCodecType::Av1, VideoProfile::Av1(_)) => {}
            (_, VideoProfile::Unknown(_)) => {}
            (codec, profile) => {
                panic!(
                    "コーデックと最大プロファイルが一致しない: codec={codec:?}, profile={profile:?}"
                )
            }
        }
    }
}

/// 解像度範囲が正の値であることを確認する
#[test]
fn test_io_caps_width_and_height_ranges_are_positive() {
    for info in supported_codecs().expect("supported_codecs が失敗した") {
        for (direction, caps) in [
            (
                "encoding input",
                info.encoding.as_ref().and_then(|info| info.input.as_ref()),
            ),
            (
                "encoding output",
                info.encoding.as_ref().and_then(|info| info.output.as_ref()),
            ),
            (
                "decoding input",
                info.decoding.as_ref().and_then(|info| info.input.as_ref()),
            ),
            (
                "decoding output",
                info.decoding.as_ref().and_then(|info| info.output.as_ref()),
            ),
        ] {
            let Some(caps) = caps else {
                continue;
            };
            assert!(
                caps.min_width > 0 && caps.max_width > 0,
                "{:?} の {} の幅が正の値ではない: {caps:?}",
                info.codec,
                direction
            );
            assert!(
                caps.min_height > 0 && caps.max_height > 0,
                "{:?} の {} の高さが正の値ではない: {caps:?}",
                info.codec,
                direction
            );
            assert!(
                caps.min_width <= caps.max_width,
                "{:?} の {} の最小幅が最大幅を超えている: {caps:?}",
                info.codec,
                direction
            );
            assert!(
                caps.min_height <= caps.max_height,
                "{:?} の {} の最小高さが最大高さを超えている: {caps:?}",
                info.codec,
                direction
            );
        }
    }
}
