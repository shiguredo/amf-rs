use shiguredo_amf::{
    Av1EncoderConfig, Av1Profile, CodecConfig, Decoder, DecoderCodec, DecoderConfig, EncodeOptions,
    EncodedFrame, Encoder, EncoderConfig, FrameFormat, H264EncoderConfig, H264Profile,
    HevcEncoderConfig, HevcProfile, PictureType, RateControlMode, ReconfigureParams, frame_type,
};

/// ダミー NV12 フレームを生成する
///
/// Y プレーンはフレーム番号に応じたグラデーション、UV プレーンは 128 固定。
fn generate_dummy_nv12(width: usize, height: usize, frame_index: usize) -> Vec<u8> {
    let y_size = width * height;
    let uv_size = y_size / 2;
    let mut data = vec![0u8; y_size + uv_size];

    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = ((x + y + frame_index * 7) % 256) as u8;
        }
    }
    for i in 0..uv_size {
        data[y_size + i] = 128;
    }

    data
}

/// SMPTE カラーバー風の NV12 フレームを生成する
///
/// 7 色の縦ストライプ（白/黄/シアン/緑/マゼンタ/赤/青）を
/// BT.601 で YUV に変換し NV12 形式で返す。
fn generate_colorbar_nv12(width: usize, height: usize) -> Vec<u8> {
    // SMPTE カラーバーの RGB 値（白/黄/シアン/緑/マゼンタ/赤/青）
    let bars: [(u8, u8, u8); 7] = [
        (235, 235, 235), // 白
        (235, 235, 16),  // 黄
        (16, 235, 235),  // シアン
        (16, 235, 16),   // 緑
        (235, 16, 235),  // マゼンタ
        (235, 16, 16),   // 赤
        (16, 16, 235),   // 青
    ];

    let y_size = width * height;
    let uv_size = y_size / 2;
    let mut data = vec![0u8; y_size + uv_size];
    let uv_offset = y_size;

    for y in 0..height {
        for x in 0..width {
            let bar_index = x * 7 / width;
            let (r, g, b) = bars[bar_index];

            // BT.601 RGB -> YCbCr
            let rf = r as f64;
            let gf = g as f64;
            let bf = b as f64;
            let yv = (0.257 * rf + 0.504 * gf + 0.098 * bf + 16.0).clamp(16.0, 235.0) as u8;
            data[y * width + x] = yv;

            // UV は 2x2 ブロック単位（左上ピクセルで代表する）
            if y % 2 == 0 && x % 2 == 0 {
                let u = (-0.148 * rf - 0.291 * gf + 0.439 * bf + 128.0).clamp(16.0, 240.0) as u8;
                let v = (0.439 * rf - 0.368 * gf - 0.071 * bf + 128.0).clamp(16.0, 240.0) as u8;
                let uv_row = y / 2;
                let uv_col = x; // NV12 はインターリーブなので x そのまま
                data[uv_offset + uv_row * width + uv_col] = u;
                data[uv_offset + uv_row * width + uv_col + 1] = v;
            }
        }
    }

    data
}

/// Y プレーン同士の PSNR を計算する（dB）
///
/// 値が大きいほど入力と出力が近い。一般に 30dB 以上あれば視覚的に良好。
fn psnr_y(original: &[u8], decoded: &[u8], width: usize, height: usize) -> f64 {
    assert_eq!(original.len(), decoded.len());
    let y_size = width * height;
    let mut mse_sum: f64 = 0.0;
    for i in 0..y_size {
        let diff = original[i] as f64 - decoded[i] as f64;
        mse_sum += diff * diff;
    }
    let mse = mse_sum / y_size as f64;
    if mse == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0_f64 * 255.0 / mse).log10()
}

/// エンコードしてフレーム一覧を返すヘルパー
///
/// 各 EncodedFrame のデータはフレーム単位で保持される。
fn encode(config: EncoderConfig, frames: &[Vec<u8>]) -> Vec<EncodedFrame> {
    let mut encoder = Encoder::new(config).expect("failed to create encoder");
    let options = EncodeOptions {
        frame_type: frame_type::UNKNOWN,
    };
    let mut encoded_frames = Vec::new();

    for frame in frames.iter() {
        encoder.encode(frame, &options).expect("failed to encode");
        while let Some(encoded) = encoder.next_frame() {
            encoded_frames.push(encoded);
        }
    }
    encoder.finish().expect("failed to finish");
    while let Some(encoded) = encoder.next_frame() {
        encoded_frames.push(encoded);
    }

    encoded_frames
}

/// Encoder のキューに溜まっている出力フレームをすべて回収する
fn collect_encoded_frames(encoder: &mut Encoder, out: &mut Vec<EncodedFrame>) {
    while let Some(encoded) = encoder.next_frame() {
        out.push(encoded);
    }
}

/// 強制キーフレームを含むシーケンスをエンコードする
fn encode_with_forced_keyframe(
    config: EncoderConfig,
    force_at: usize,
    num_frames: usize,
) -> Vec<EncodedFrame> {
    let width = config.width as usize;
    let height = config.height as usize;
    let mut encoder = Encoder::new(config).expect("failed to create encoder");
    let mut encoded_frames = Vec::new();

    for i in 0..num_frames {
        let frame = generate_dummy_nv12(width, height, i);
        let options = if i == force_at {
            EncodeOptions {
                frame_type: frame_type::IDR | frame_type::I | frame_type::REF,
            }
        } else {
            EncodeOptions {
                frame_type: frame_type::UNKNOWN,
            }
        };
        encoder.encode(&frame, &options).expect("failed to encode");
        collect_encoded_frames(&mut encoder, &mut encoded_frames);
    }
    encoder.finish().expect("failed to finish");
    collect_encoded_frames(&mut encoder, &mut encoded_frames);

    encoded_frames
}

/// n 回目の IDR フレームのインデックスを返す（n は 1 始まり）
fn nth_idr_frame_index(encoded_frames: &[EncodedFrame], nth: usize) -> usize {
    let mut count = 0usize;
    for (i, frame) in encoded_frames.iter().enumerate() {
        if frame.picture_type() == PictureType::Idr {
            count += 1;
            if count == nth {
                return i;
            }
        }
    }
    panic!("expected at least {nth} IDR frames, got {count}");
}

/// Annex-B のスタートコードを検索する
fn find_annex_b_start_code(data: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 3 <= data.len() {
        if i + 4 <= data.len() && data[i..i + 4] == [0, 0, 0, 1] {
            return Some((i, 4));
        }
        if data[i..i + 3] == [0, 0, 1] {
            return Some((i, 3));
        }
        i += 1;
    }
    None
}

/// H.264 ビットストリーム内に SPS と PPS が含まれているかを判定する
fn h264_contains_sps_pps(data: &[u8]) -> bool {
    let mut has_sps = false;
    let mut has_pps = false;
    let mut pos = 0usize;

    while let Some((sc, sc_len)) = find_annex_b_start_code(data, pos) {
        let nal_start = sc + sc_len;
        let nal_end = find_annex_b_start_code(data, nal_start)
            .map(|(idx, _)| idx)
            .unwrap_or(data.len());
        if nal_start < nal_end {
            let nal_type = data[nal_start] & 0x1f;
            has_sps |= nal_type == 7;
            has_pps |= nal_type == 8;
        }
        pos = nal_start;
    }

    has_sps && has_pps
}

/// HEVC ビットストリーム内に VPS / SPS / PPS が含まれているかを判定する
fn hevc_contains_vps_sps_pps(data: &[u8]) -> bool {
    let mut has_vps = false;
    let mut has_sps = false;
    let mut has_pps = false;
    let mut pos = 0usize;

    while let Some((sc, sc_len)) = find_annex_b_start_code(data, pos) {
        let nal_start = sc + sc_len;
        let nal_end = find_annex_b_start_code(data, nal_start)
            .map(|(idx, _)| idx)
            .unwrap_or(data.len());
        if nal_start + 1 < nal_end {
            let nal_type = (data[nal_start] >> 1) & 0x3f;
            has_vps |= nal_type == 32;
            has_sps |= nal_type == 33;
            has_pps |= nal_type == 34;
        }
        pos = nal_start;
    }

    has_vps && has_sps && has_pps
}

/// AV1 OBU の leb128 サイズを読み取る
fn read_av1_leb128(data: &[u8], pos: &mut usize) -> Option<usize> {
    let mut value = 0usize;
    let mut shift = 0usize;
    for _ in 0..8 {
        if *pos >= data.len() {
            return None;
        }
        let byte = data[*pos];
        *pos += 1;
        value |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
    }
    None
}

/// AV1 ビットストリーム内に Sequence Header OBU が含まれているかを判定する
fn av1_contains_sequence_header_obu(data: &[u8]) -> bool {
    let mut pos = 0usize;
    while pos < data.len() {
        let obu_header = data[pos];
        pos += 1;

        let obu_type = (obu_header >> 3) & 0x0f;
        let has_extension = (obu_header & 0b0000_0100) != 0;
        let has_size_field = (obu_header & 0b0000_0010) != 0;
        if has_extension {
            if pos >= data.len() {
                return false;
            }
            pos += 1;
        }

        let payload_size = if has_size_field {
            match read_av1_leb128(data, &mut pos) {
                Some(v) => v,
                None => return false,
            }
        } else {
            data.len() - pos
        };
        if pos + payload_size > data.len() {
            return false;
        }
        if obu_type == 1 {
            return true;
        }
        pos += payload_size;
        if !has_size_field {
            break;
        }
    }

    false
}

/// エンコード済みフレームをフレーム単位でデコードして返すヘルパー
///
/// AMF デコーダーはフレーム単位でデータを受け取る必要がある。
fn decode(
    decoder_codec: DecoderCodec,
    encoded_frames: &[EncodedFrame],
) -> Vec<shiguredo_amf::DecodedFrame> {
    let config = DecoderConfig {
        codec: decoder_codec,
    };
    let mut decoder = Decoder::new(config).expect("failed to create decoder");

    // フレーム単位でデコーダーに送信する
    for encoded in encoded_frames {
        decoder.decode(encoded.data()).expect("failed to decode");
    }

    decoder.finish().expect("failed to finish");

    let mut decoded_frames = Vec::new();
    while let Some(frame) = decoder.next_frame() {
        decoded_frames.push(frame);
    }

    decoded_frames
}

/// エンコード→デコードのラウンドトリップを検証するヘルパー
fn roundtrip(
    encoder_config: EncoderConfig,
    decoder_codec: DecoderCodec,
    input_frames: &[Vec<u8>],
) -> (Vec<EncodedFrame>, Vec<shiguredo_amf::DecodedFrame>) {
    let width = encoder_config.width as usize;
    let height = encoder_config.height as usize;
    let num_frames = input_frames.len();

    let encoded_frames = encode(encoder_config, input_frames);

    assert!(
        !encoded_frames.is_empty(),
        "no encoded frames were produced"
    );
    for (i, frame) in encoded_frames.iter().enumerate() {
        assert!(!frame.data().is_empty(), "encoded frame {i} has empty data");
    }

    let decoded_frames = decode(decoder_codec, &encoded_frames);

    assert_eq!(
        decoded_frames.len(),
        num_frames,
        "decoded {} frames, expected {num_frames}",
        decoded_frames.len()
    );
    for (i, frame) in decoded_frames.iter().enumerate() {
        assert_eq!(frame.width(), width, "decoded frame {i} width mismatch");
        assert_eq!(frame.height(), height, "decoded frame {i} height mismatch");
        assert!(!frame.data().is_empty(), "decoded frame {i} has empty data");
    }

    (encoded_frames, decoded_frames)
}

/// カラーバーを使ったラウンドトリップで PSNR を検証するヘルパー
///
/// 同一のカラーバーフレームを num_frames 回エンコードし、デコード後に
/// 元の Y プレーンとの PSNR が min_psnr_db 以上であることを確認する。
fn roundtrip_colorbar(
    encoder_config: EncoderConfig,
    decoder_codec: DecoderCodec,
    num_frames: usize,
    min_psnr_db: f64,
) {
    let width = encoder_config.width as usize;
    let height = encoder_config.height as usize;

    let colorbar = generate_colorbar_nv12(width, height);
    let input_frames: Vec<Vec<u8>> = (0..num_frames).map(|_| colorbar.clone()).collect();

    let (_, decoded_frames) = roundtrip(encoder_config, decoder_codec, &input_frames);

    for (i, decoded) in decoded_frames.iter().enumerate() {
        let psnr = psnr_y(&colorbar, decoded.data(), width, height);
        assert!(
            psnr >= min_psnr_db,
            "frame {i}: PSNR {psnr:.1} dB < {min_psnr_db} dB"
        );
    }
}

// --- H.264 ---

/// H.264 CBR カラーバーのラウンドトリップ（PSNR 検証）
#[test]
fn test_roundtrip_h264_cbr() {
    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    config.gop_pic_size = Some(30);

    roundtrip_colorbar(config, DecoderCodec::H264, 30, 25.0);
}

/// H.264 CQP カラーバーのラウンドトリップ（PSNR 検証）
#[test]
fn test_roundtrip_h264_cqp() {
    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cqp,
    );
    config.qpi = Some(26);
    config.qpp = Some(28);
    config.qpb = Some(30);
    config.gop_pic_size = Some(10);

    roundtrip_colorbar(config, DecoderCodec::H264, 10, 25.0);
}

/// H.264 で IDR フレームを強制してラウンドトリップする
#[test]
fn test_roundtrip_h264_force_idr() {
    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    config.gop_pic_size = Some(300);

    let width = config.width as usize;
    let height = config.height as usize;
    let mut encoder = Encoder::new(config).expect("failed to create encoder");
    let mut encoded_frames = Vec::new();

    for i in 0..15 {
        let frame = generate_dummy_nv12(width, height, i);
        let options = if i == 10 {
            EncodeOptions {
                frame_type: frame_type::IDR | frame_type::I | frame_type::REF,
            }
        } else {
            EncodeOptions {
                frame_type: frame_type::UNKNOWN,
            }
        };
        encoder.encode(&frame, &options).expect("failed to encode");
        while let Some(encoded) = encoder.next_frame() {
            encoded_frames.push(encoded);
        }
    }
    encoder.finish().expect("failed to finish");
    while let Some(encoded) = encoder.next_frame() {
        encoded_frames.push(encoded);
    }

    let idr_count = encoded_frames
        .iter()
        .filter(|f| f.picture_type() == PictureType::Idr)
        .count();
    assert!(
        idr_count >= 2,
        "expected at least 2 IDR frames, got {idr_count}"
    );

    // フレーム単位でデコードして復号できることを確認する
    let decoded_frames = decode(DecoderCodec::H264, &encoded_frames);
    assert_eq!(decoded_frames.len(), 15);
}

/// H.264 で force_picture_type 指定時に IDR と SPS / PPS が出力されることを確認する
#[test]
fn test_force_picture_type_h264_outputs_idr_and_sps_pps() {
    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    config.gop_pic_size = Some(300);

    let encoded_frames = encode_with_forced_keyframe(config, 10, 15);
    let forced_idr_index = nth_idr_frame_index(&encoded_frames, 2);
    let forced_idr = &encoded_frames[forced_idr_index];

    assert_eq!(forced_idr.picture_type(), PictureType::Idr);
    assert!(
        h264_contains_sps_pps(forced_idr.data()),
        "forced H.264 IDR frame must include SPS and PPS"
    );
}

// --- H.265 ---

/// H.265 CBR カラーバーのラウンドトリップ（PSNR 検証）
#[test]
fn test_roundtrip_hevc_cbr() {
    let mut config = EncoderConfig::new(
        CodecConfig::Hevc(HevcEncoderConfig {
            profile: Some(HevcProfile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    config.gop_pic_size = Some(30);

    roundtrip_colorbar(config, DecoderCodec::Hevc, 30, 25.0);
}

/// H.265 CQP カラーバーのラウンドトリップ（PSNR 検証）
#[test]
fn test_roundtrip_hevc_cqp() {
    let mut config = EncoderConfig::new(
        CodecConfig::Hevc(HevcEncoderConfig {
            profile: Some(HevcProfile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cqp,
    );
    config.qpi = Some(26);
    config.qpp = Some(28);
    config.qpb = Some(30);
    config.gop_pic_size = Some(10);

    roundtrip_colorbar(config, DecoderCodec::Hevc, 10, 25.0);
}

/// HEVC で force_picture_type 指定時に IDR と VPS / SPS / PPS が出力されることを確認する
#[test]
fn test_force_picture_type_hevc_outputs_idr_and_vps_sps_pps() {
    let mut config = EncoderConfig::new(
        CodecConfig::Hevc(HevcEncoderConfig {
            profile: Some(HevcProfile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    config.gop_pic_size = Some(300);

    let encoded_frames = encode_with_forced_keyframe(config, 10, 15);
    let forced_idr_index = nth_idr_frame_index(&encoded_frames, 2);
    let forced_idr = &encoded_frames[forced_idr_index];

    assert_eq!(forced_idr.picture_type(), PictureType::Idr);
    assert!(
        hevc_contains_vps_sps_pps(forced_idr.data()),
        "forced HEVC IDR frame must include VPS, SPS and PPS"
    );
}

// --- AV1 ---

/// AV1 CBR カラーバーのラウンドトリップ（PSNR 検証）
#[test]
fn test_roundtrip_av1_cbr() {
    let mut config = EncoderConfig::new(
        CodecConfig::Av1(Av1EncoderConfig {
            profile: Some(Av1Profile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    config.gop_pic_size = Some(30);

    roundtrip_colorbar(config, DecoderCodec::Av1, 30, 25.0);
}

/// AV1 CQP カラーバーのラウンドトリップ（PSNR 検証）
#[test]
fn test_roundtrip_av1_cqp() {
    let mut config = EncoderConfig::new(
        CodecConfig::Av1(Av1EncoderConfig {
            profile: Some(Av1Profile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cqp,
    );
    config.qpi = Some(26);
    config.qpp = Some(28);
    config.qpb = Some(30);
    config.gop_pic_size = Some(10);

    roundtrip_colorbar(config, DecoderCodec::Av1, 10, 25.0);
}

/// AV1 で force_picture_type 指定時にキーフレームと Sequence Header が出力されることを確認する
#[test]
fn test_force_picture_type_av1_outputs_keyframe_and_sequence_header() {
    let mut config = EncoderConfig::new(
        CodecConfig::Av1(Av1EncoderConfig {
            profile: Some(Av1Profile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    config.gop_pic_size = Some(300);

    let encoded_frames = encode_with_forced_keyframe(config, 10, 15);
    let forced_key_index = nth_idr_frame_index(&encoded_frames, 2);
    let forced_key = &encoded_frames[forced_key_index];

    assert_eq!(forced_key.picture_type(), PictureType::Idr);
    assert!(
        av1_contains_sequence_header_obu(forced_key.data()),
        "forced AV1 key frame must include sequence header OBU"
    );
}

/// reconfigure で 0 を含むフレームレートを指定するとエラーになることを確認する
#[test]
fn test_reconfigure_invalid_framerate_zero() {
    let config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    let mut encoder = Encoder::new(config).expect("failed to create encoder");

    let err = encoder
        .reconfigure(ReconfigureParams {
            framerate_num: Some(0),
            framerate_den: Some(1),
            ..ReconfigureParams::default()
        })
        .expect_err("reconfigure should fail for zero framerate");
    assert!(
        err.to_string().contains("invalid framerate"),
        "unexpected error: {err}"
    );
}

/// H.264 でエンコード途中にビットレートとフレームレートを再設定できることを確認する
#[test]
fn test_reconfigure_h264_runtime_change() {
    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    let mut encoder = Encoder::new(config).expect("failed to create encoder");
    let options = EncodeOptions {
        frame_type: frame_type::UNKNOWN,
    };
    let mut encoded_frames = Vec::new();

    for i in 0..5 {
        let frame = generate_dummy_nv12(320, 240, i);
        encoder.encode(&frame, &options).expect("failed to encode");
        collect_encoded_frames(&mut encoder, &mut encoded_frames);
    }

    encoder
        .reconfigure(ReconfigureParams {
            framerate_num: Some(15),
            framerate_den: Some(1),
            target_kbps: Some(1500),
            ..ReconfigureParams::default()
        })
        .expect("failed to reconfigure");

    for i in 5..10 {
        let frame = generate_dummy_nv12(320, 240, i);
        encoder.encode(&frame, &options).expect("failed to encode");
        collect_encoded_frames(&mut encoder, &mut encoded_frames);
    }

    encoder.finish().expect("failed to finish");
    collect_encoded_frames(&mut encoder, &mut encoded_frames);
    assert!(!encoded_frames.is_empty(), "no encoded frames produced");

    let decoded = decode(DecoderCodec::H264, &encoded_frames);
    assert_eq!(decoded.len(), 10);
}

/// HEVC で非対応項目 qpb/gop_pic_size を指定しても失敗せず継続できることを確認する
#[test]
fn test_reconfigure_hevc_ignores_qpb_and_gop() {
    let mut config = EncoderConfig::new(
        CodecConfig::Hevc(HevcEncoderConfig {
            profile: Some(HevcProfile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    let mut encoder = Encoder::new(config).expect("failed to create encoder");
    let options = EncodeOptions {
        frame_type: frame_type::UNKNOWN,
    };
    let mut encoded_frames = Vec::new();

    for i in 0..4 {
        let frame = generate_dummy_nv12(320, 240, i);
        encoder.encode(&frame, &options).expect("failed to encode");
        collect_encoded_frames(&mut encoder, &mut encoded_frames);
    }

    encoder
        .reconfigure(ReconfigureParams {
            framerate_num: Some(24),
            framerate_den: Some(1),
            target_kbps: Some(1200),
            qpi: Some(24),
            qpp: Some(26),
            qpb: Some(28),
            gop_pic_size: Some(20),
            ..ReconfigureParams::default()
        })
        .expect("failed to reconfigure");

    for i in 4..8 {
        let frame = generate_dummy_nv12(320, 240, i);
        encoder.encode(&frame, &options).expect("failed to encode");
        collect_encoded_frames(&mut encoder, &mut encoded_frames);
    }

    encoder.finish().expect("failed to finish");
    collect_encoded_frames(&mut encoder, &mut encoded_frames);
    assert!(!encoded_frames.is_empty(), "no encoded frames produced");

    let decoded = decode(DecoderCodec::Hevc, &encoded_frames);
    assert_eq!(decoded.len(), 8);
}

/// AV1 で qpb と gop_pic_size の再設定を適用して継続できることを確認する
#[test]
fn test_reconfigure_av1_qpb_and_gop() {
    let mut config = EncoderConfig::new(
        CodecConfig::Av1(Av1EncoderConfig {
            profile: Some(Av1Profile::Main),
        }),
        320,
        240,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    let mut encoder = Encoder::new(config).expect("failed to create encoder");
    let options = EncodeOptions {
        frame_type: frame_type::UNKNOWN,
    };
    let mut encoded_frames = Vec::new();

    for i in 0..4 {
        let frame = generate_dummy_nv12(320, 240, i);
        encoder.encode(&frame, &options).expect("failed to encode");
        collect_encoded_frames(&mut encoder, &mut encoded_frames);
    }

    encoder
        .reconfigure(ReconfigureParams {
            framerate_num: Some(24),
            framerate_den: Some(1),
            target_kbps: Some(1300),
            qpi: Some(96),
            qpp: Some(104),
            qpb: Some(112),
            gop_pic_size: Some(16),
            ..ReconfigureParams::default()
        })
        .expect("failed to reconfigure");

    for i in 4..8 {
        let frame = generate_dummy_nv12(320, 240, i);
        encoder.encode(&frame, &options).expect("failed to encode");
        collect_encoded_frames(&mut encoder, &mut encoded_frames);
    }

    encoder.finish().expect("failed to finish");
    collect_encoded_frames(&mut encoder, &mut encoded_frames);
    assert!(!encoded_frames.is_empty(), "no encoded frames produced");

    let decoded = decode(DecoderCodec::Av1, &encoded_frames);
    assert_eq!(decoded.len(), 8);
}

// ---------------------------------------------------------------------------
// 各フレームフォーマットのダミーデータ生成ヘルパー
// ---------------------------------------------------------------------------

/// ダミー I420 フレームを生成する
///
/// Planar YUV 4:2:0: Y プレーン + U プレーン + V プレーン
fn generate_dummy_i420(width: usize, height: usize, frame_index: usize) -> Vec<u8> {
    let y_size = width * height;
    let uv_size = (width / 2) * (height / 2);
    let mut data = vec![0u8; y_size + uv_size * 2];

    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = ((x + y + frame_index * 7) % 256) as u8;
        }
    }
    // U プレーン
    for i in 0..uv_size {
        data[y_size + i] = 128;
    }
    // V プレーン
    for i in 0..uv_size {
        data[y_size + uv_size + i] = 128;
    }

    data
}

/// ダミー YV12 フレームを生成する
///
/// Planar YUV 4:2:0: Y プレーン + V プレーン + U プレーン（I420 と UV が逆順）
fn generate_dummy_yv12(width: usize, height: usize, frame_index: usize) -> Vec<u8> {
    let y_size = width * height;
    let uv_size = (width / 2) * (height / 2);
    let mut data = vec![0u8; y_size + uv_size * 2];

    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = ((x + y + frame_index * 7) % 256) as u8;
        }
    }
    // V プレーン
    for i in 0..uv_size {
        data[y_size + i] = 128;
    }
    // U プレーン
    for i in 0..uv_size {
        data[y_size + uv_size + i] = 128;
    }

    data
}

/// ダミー BGRA フレームを生成する
fn generate_dummy_bgra(width: usize, height: usize, frame_index: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            let v = ((x + y + frame_index * 7) % 256) as u8;
            data[offset] = v; // B
            data[offset + 1] = v; // G
            data[offset + 2] = v; // R
            data[offset + 3] = 255; // A
        }
    }
    data
}

/// ダミー ARGB フレームを生成する
fn generate_dummy_argb(width: usize, height: usize, frame_index: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            let v = ((x + y + frame_index * 7) % 256) as u8;
            data[offset] = 255; // A
            data[offset + 1] = v; // R
            data[offset + 2] = v; // G
            data[offset + 3] = v; // B
        }
    }
    data
}

/// ダミー RGBA フレームを生成する
fn generate_dummy_rgba(width: usize, height: usize, frame_index: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            let v = ((x + y + frame_index * 7) % 256) as u8;
            data[offset] = v; // R
            data[offset + 1] = v; // G
            data[offset + 2] = v; // B
            data[offset + 3] = 255; // A
        }
    }
    data
}

/// ダミー YUY2 フレームを生成する
///
/// Packed YUV 4:2:2: [Y0 U0 Y1 V0] の繰り返し
fn generate_dummy_yuy2(width: usize, height: usize, frame_index: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 2];
    for y in 0..height {
        for x in (0..width).step_by(2) {
            let offset = (y * width + x) * 2;
            let v = ((x + y + frame_index * 7) % 256) as u8;
            data[offset] = v; // Y0
            data[offset + 1] = 128; // U
            data[offset + 2] = v; // Y1
            data[offset + 3] = 128; // V
        }
    }
    data
}

/// 指定フォーマットのダミーフレームを生成する
fn generate_dummy_frame(
    format: FrameFormat,
    width: usize,
    height: usize,
    frame_index: usize,
) -> Vec<u8> {
    match format {
        FrameFormat::Nv12 => generate_dummy_nv12(width, height, frame_index),
        FrameFormat::I420 => generate_dummy_i420(width, height, frame_index),
        FrameFormat::Yv12 => generate_dummy_yv12(width, height, frame_index),
        FrameFormat::Bgra => generate_dummy_bgra(width, height, frame_index),
        FrameFormat::Argb => generate_dummy_argb(width, height, frame_index),
        FrameFormat::Rgba => generate_dummy_rgba(width, height, frame_index),
        FrameFormat::Yuy2 => generate_dummy_yuy2(width, height, frame_index),
        _ => unimplemented!("{format:?} のダミーフレーム生成は未実装"),
    }
}

/// フォーマット指定のラウンドトリップテストを実行するヘルパー
///
/// 指定フォーマットでダミーフレームを生成し、エンコード→デコードして
/// フレーム数が一致することを検証する。
fn roundtrip_format(
    codec_config: CodecConfig,
    decoder_codec: DecoderCodec,
    format: FrameFormat,
    num_frames: usize,
) {
    let width: u32 = 320;
    let height: u32 = 240;

    let mut config = EncoderConfig::new(
        codec_config,
        width,
        height,
        format,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);
    config.gop_pic_size = Some(30);

    let input_frames: Vec<Vec<u8>> = (0..num_frames)
        .map(|i| generate_dummy_frame(format, width as usize, height as usize, i))
        .collect();

    // フレームサイズが FrameFormat::frame_size() と一致することを確認する
    let expected_size = format
        .frame_size(width as usize, height as usize)
        .expect("frame size overflow");
    for (i, frame) in input_frames.iter().enumerate() {
        assert_eq!(
            frame.len(),
            expected_size,
            "frame {i}: size {} != expected {expected_size} for {format:?}",
            frame.len()
        );
    }

    let (_, decoded_frames) = roundtrip(config, decoder_codec, &input_frames);

    assert_eq!(
        decoded_frames.len(),
        num_frames,
        "{format:?}: decoded {} frames, expected {num_frames}",
        decoded_frames.len()
    );
}

// ---------------------------------------------------------------------------
// フレームフォーマット別ラウンドトリップテスト (H.264 High)
// ---------------------------------------------------------------------------

/// I420 入力の H.264 ラウンドトリップ
#[test]
fn test_roundtrip_h264_i420() {
    roundtrip_format(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        DecoderCodec::H264,
        FrameFormat::I420,
        10,
    );
}

/// YV12 入力の H.264 ラウンドトリップ
#[test]
fn test_roundtrip_h264_yv12() {
    roundtrip_format(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        DecoderCodec::H264,
        FrameFormat::Yv12,
        10,
    );
}

/// BGRA 入力の H.264 ラウンドトリップ
#[test]
fn test_roundtrip_h264_bgra() {
    roundtrip_format(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        DecoderCodec::H264,
        FrameFormat::Bgra,
        10,
    );
}

/// ARGB 入力の H.264 ラウンドトリップ
#[test]
fn test_roundtrip_h264_argb() {
    roundtrip_format(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        DecoderCodec::H264,
        FrameFormat::Argb,
        10,
    );
}

/// RGBA 入力の H.264 ラウンドトリップ
#[test]
fn test_roundtrip_h264_rgba() {
    roundtrip_format(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        DecoderCodec::H264,
        FrameFormat::Rgba,
        10,
    );
}

/// YUY2 入力の H.264 ラウンドトリップ
#[test]
fn test_roundtrip_h264_yuy2() {
    roundtrip_format(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        DecoderCodec::H264,
        FrameFormat::Yuy2,
        10,
    );
}

// ---------------------------------------------------------------------------
// 4K 高ビットレートラウンドトリップテスト
// ---------------------------------------------------------------------------

/// 4K H.264 CBR 20Mbps のラウンドトリップ
#[test]
fn test_roundtrip_h264_4k_high_bitrate() {
    let width: u32 = 3840;
    let height: u32 = 2160;
    let num_frames = 5;

    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        width,
        height,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(20_000);
    config.gop_pic_size = Some(30);

    let colorbar = generate_colorbar_nv12(width as usize, height as usize);
    let input_frames: Vec<Vec<u8>> = (0..num_frames).map(|_| colorbar.clone()).collect();

    let (_, decoded_frames) = roundtrip(config, DecoderCodec::H264, &input_frames);

    for (i, decoded) in decoded_frames.iter().enumerate() {
        let psnr = psnr_y(&colorbar, decoded.data(), width as usize, height as usize);
        assert!(psnr >= 25.0, "frame {i}: PSNR {psnr:.1} dB < 25.0 dB");
    }
}

/// 4K H.265 CBR 20Mbps のラウンドトリップ
#[test]
fn test_roundtrip_hevc_4k_high_bitrate() {
    let width: u32 = 3840;
    let height: u32 = 2160;
    let num_frames = 5;

    let mut config = EncoderConfig::new(
        CodecConfig::Hevc(HevcEncoderConfig {
            profile: Some(HevcProfile::Main),
        }),
        width,
        height,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(20_000);
    config.gop_pic_size = Some(30);

    let colorbar = generate_colorbar_nv12(width as usize, height as usize);
    let input_frames: Vec<Vec<u8>> = (0..num_frames).map(|_| colorbar.clone()).collect();

    let (_, decoded_frames) = roundtrip(config, DecoderCodec::Hevc, &input_frames);

    for (i, decoded) in decoded_frames.iter().enumerate() {
        let psnr = psnr_y(&colorbar, decoded.data(), width as usize, height as usize);
        assert!(psnr >= 25.0, "frame {i}: PSNR {psnr:.1} dB < 25.0 dB");
    }
}

/// 4K AV1 CBR 20Mbps のラウンドトリップ
#[test]
fn test_roundtrip_av1_4k_high_bitrate() {
    let width: u32 = 3840;
    let height: u32 = 2160;
    let num_frames = 5;

    let mut config = EncoderConfig::new(
        CodecConfig::Av1(Av1EncoderConfig {
            profile: Some(Av1Profile::Main),
        }),
        width,
        height,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(20_000);
    config.gop_pic_size = Some(30);

    let colorbar = generate_colorbar_nv12(width as usize, height as usize);
    let input_frames: Vec<Vec<u8>> = (0..num_frames).map(|_| colorbar.clone()).collect();

    let (_, decoded_frames) = roundtrip(config, DecoderCodec::Av1, &input_frames);

    for (i, decoded) in decoded_frames.iter().enumerate() {
        let psnr = psnr_y(&colorbar, decoded.data(), width as usize, height as usize);
        assert!(psnr >= 25.0, "frame {i}: PSNR {psnr:.1} dB < 25.0 dB");
    }
}
