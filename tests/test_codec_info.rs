use shiguredo_amf::{
    AccelerationType, Av1Profile, H264Profile, HevcProfile, VideoCodecType, VideoProfile,
    supported_codecs,
};

/// `supported_codecs` が全コーデックの能力情報を返すことを確認する
#[test]
fn test_supported_codecs_returns_all_codecs() {
    let codecs = supported_codecs().expect("supported_codecs が失敗した");
    assert_eq!(codecs.len(), 4, "コーデック数が 4 ではない: {codecs:?}");

    for codec in [
        VideoCodecType::H264,
        VideoCodecType::Hevc,
        VideoCodecType::Av1,
        VideoCodecType::Vp9,
    ] {
        let info = codecs
            .iter()
            .find(|info| info.codec == codec)
            .unwrap_or_else(|| panic!("{codec:?} が含まれない: {codecs:?}"));
        let decoding = info
            .decoding
            .as_ref()
            .unwrap_or_else(|| panic!("{codec:?} のデコード能力がない"));
        assert_eq!(
            decoding.acceleration,
            Some(AccelerationType::Hardware),
            "{codec:?} のデコード acceleration が期待値と一致しない"
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

        // VP9 は AMF にエンコーダーがないため、エンコード能力は None になる。
        if codec == VideoCodecType::Vp9 {
            assert!(info.encoding.is_none(), "{codec:?} のエンコード能力がある");
            continue;
        }

        let encoding = info
            .encoding
            .as_ref()
            .unwrap_or_else(|| panic!("{codec:?} のエンコード能力がない"));
        assert_eq!(
            encoding.acceleration,
            Some(AccelerationType::Hardware),
            "{codec:?} のエンコード acceleration が期待値と一致しない"
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
}

/// エンコーダーの能力情報が実機の既知値と一致することを確認する
///
/// VP9 は AMF にエンコーダーがないため、エンコード能力が None であることを確認する。
#[test]
fn test_encoding_profile_matches_codec() {
    for info in supported_codecs().expect("supported_codecs が失敗した") {
        let expected = match info.codec {
            VideoCodecType::H264 => Some((VideoProfile::H264(H264Profile::High), 52, Some(true))),
            VideoCodecType::Hevc => Some((VideoProfile::Hevc(HevcProfile::Main10), 186, None)),
            VideoCodecType::Av1 => Some((VideoProfile::Av1(Av1Profile::Main), 16, Some(false))),
            VideoCodecType::Vp9 => None,
        };
        let Some((profile, level, frame_reordering)) = expected else {
            assert!(
                info.encoding.is_none(),
                "{:?} のエンコード能力がある",
                info.codec
            );
            continue;
        };
        let encoding = info
            .encoding
            .as_ref()
            .unwrap_or_else(|| panic!("{:?} のエンコード能力がない", info.codec));
        assert_eq!(
            encoding.acceleration,
            Some(AccelerationType::Hardware),
            "{:?} の acceleration が期待値と一致しない",
            info.codec
        );
        assert_eq!(
            encoding.max_profile,
            Some(profile),
            "{:?} の max_profile が期待値と一致しない",
            info.codec
        );
        assert_eq!(
            encoding.max_level,
            Some(level),
            "{:?} の max_level が期待値と一致しない",
            info.codec
        );
        assert_eq!(
            encoding.supports_frame_reordering, frame_reordering,
            "{:?} の supports_frame_reordering が期待値と一致しない",
            info.codec
        );
    }
}

/// 解像度範囲が正の値であることを確認する
#[test]
fn test_io_capabilities_width_and_height_ranges_are_positive() {
    for info in supported_codecs().expect("supported_codecs が失敗した") {
        let mut directions = vec![
            (
                "decoding input",
                info.decoding.as_ref().and_then(|info| info.input.as_ref()),
            ),
            (
                "decoding output",
                info.decoding.as_ref().and_then(|info| info.output.as_ref()),
            ),
        ];
        if let Some(encoding) = &info.encoding {
            directions.push(("encoding input", encoding.input.as_ref()));
            directions.push(("encoding output", encoding.output.as_ref()));
        }

        for (direction, caps) in directions {
            let caps = caps
                .unwrap_or_else(|| panic!("{:?} の {} の能力情報がない", info.codec, direction));
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
