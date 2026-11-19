use std::sync::{Arc, Mutex};

use shiguredo_amf::amf::{Context, Surface};
use shiguredo_amf::{
    Av1EncoderConfig, Av1Profile, CodecConfig, Decoder, DecoderCodec, DecoderConfig, EncodeOptions,
    EncodedFrame, Encoder, EncoderConfig, Error, FnDecodeHandler, FnEncodeHandler, FrameFormat,
    H264EncoderConfig, H264Profile, HevcEncoderConfig, HevcProfile, MemoryType, PictureType,
    PlaneType, RateControlMode, ReconfigureParams, SurfaceFormat, VideoCodecType, frame_type,
    supported_codecs,
};

pub mod helpers;

use helpers::{
    copy_nv12_to_surface, create_context, generate_dummy_nv12, h264_cbr_test_encoder_config,
};

/// I420/YV12 データを Surface にコピーする
fn copy_i420_to_surface(surface: &Surface, data: &[u8], width: usize, height: usize) {
    let y_size = width * height;
    let uv_plane_size = (width / 2) * (height / 2);
    let uv_width = width / 2;
    let uv_height = height / 2;

    // Y プレーン
    let mut y_plane = surface
        .get_plane(PlaneType::Y)
        .expect("Y プレーンの取得に失敗");
    let y_hpitch = y_plane.get_hpitch() as usize;
    let y_data = unsafe { y_plane.as_mut_slice() }.expect("Y プレーンのスライス取得に失敗");
    for row in 0..height {
        let src = &data[row * width..row * width + width];
        let dst = &mut y_data[row * y_hpitch..row * y_hpitch + width];
        dst.copy_from_slice(src);
    }

    // U プレーン
    let mut u_plane = surface
        .get_plane(PlaneType::U)
        .expect("U プレーンの取得に失敗");
    let u_hpitch = u_plane.get_hpitch() as usize;
    let u_data = unsafe { u_plane.as_mut_slice() }.expect("U プレーンのスライス取得に失敗");
    for row in 0..uv_height {
        let src = &data[y_size + row * uv_width..y_size + row * uv_width + uv_width];
        let dst = &mut u_data[row * u_hpitch..row * u_hpitch + uv_width];
        dst.copy_from_slice(src);
    }

    // V プレーン
    let mut v_plane = surface
        .get_plane(PlaneType::V)
        .expect("V プレーンの取得に失敗");
    let v_hpitch = v_plane.get_hpitch() as usize;
    let v_data = unsafe { v_plane.as_mut_slice() }.expect("V プレーンのスライス取得に失敗");
    for row in 0..uv_height {
        let start = y_size + uv_plane_size + row * uv_width;
        let src = &data[start..start + uv_width];
        let dst = &mut v_data[row * v_hpitch..row * v_hpitch + uv_width];
        dst.copy_from_slice(src);
    }
}

/// Packed データを Surface にコピーする
fn copy_packed_to_surface(surface: &Surface, data: &[u8], width: usize, height: usize, bpp: usize) {
    let mut plane = surface
        .get_plane_at(0)
        .expect("インデックス 0 のプレーンの取得に失敗");
    let hpitch = plane.get_hpitch() as usize;
    let plane_data =
        unsafe { plane.as_mut_slice() }.expect("インデックス 0 のプレーンのスライス取得に失敗");
    let row_bytes = width * bpp;
    for row in 0..height {
        let src = &data[row * row_bytes..row * row_bytes + row_bytes];
        let dst = &mut plane_data[row * hpitch..row * hpitch + row_bytes];
        dst.copy_from_slice(src);
    }
}

/// 指定フォーマットのフレームデータを Surface にコピーする
fn copy_frame_to_surface(
    surface: &Surface,
    data: &[u8],
    format: FrameFormat,
    width: usize,
    height: usize,
) {
    match format {
        FrameFormat::Nv12 => copy_nv12_to_surface(surface, data, width, height),
        FrameFormat::I420 | FrameFormat::Yv12 => copy_i420_to_surface(surface, data, width, height),
        FrameFormat::Bgra
        | FrameFormat::Argb
        | FrameFormat::Rgba
        | FrameFormat::Ayuv
        | FrameFormat::Y410
        | FrameFormat::Y210 => copy_packed_to_surface(surface, data, width, height, 4),
        FrameFormat::Y416 => copy_packed_to_surface(surface, data, width, height, 8),
        FrameFormat::Yuy2 | FrameFormat::Uyvy => {
            copy_packed_to_surface(surface, data, width, height, 2)
        }
        _ => unimplemented!("{format:?} のコピーは未実装"),
    }
}

/// Surface から Y プレーンデータを読み取る
fn read_y_plane_from_surface(surface: &Surface, width: usize, height: usize) -> Vec<u8> {
    let y_plane = surface
        .get_plane(PlaneType::Y)
        .expect("Y プレーンの取得に失敗");
    let y_hpitch = y_plane.get_hpitch() as usize;
    let y_data = unsafe { y_plane.as_slice() }.expect("Y プレーンのスライス取得に失敗");
    let mut result = vec![0u8; width * height];
    for row in 0..height {
        let src = &y_data[row * y_hpitch..row * y_hpitch + width];
        let dst = &mut result[row * width..row * width + width];
        dst.copy_from_slice(src);
    }
    result
}

/// Buffer からデータを読み取る
fn buffer_to_slice(buffer: &shiguredo_amf::amf::Buffer) -> &[u8] {
    unsafe { buffer.as_slice() }.expect("バッファのスライス取得に失敗")
}

/// Surface から幅を取得する
fn surface_width(surface: &shiguredo_amf::amf::Surface) -> usize {
    surface
        .get_plane(PlaneType::Y)
        .map(|p| p.get_width() as usize)
        .unwrap_or(0)
}

/// Surface から高さを取得する
fn surface_height(surface: &shiguredo_amf::amf::Surface) -> usize {
    surface
        .get_plane(PlaneType::Y)
        .map(|p| p.get_height() as usize)
        .unwrap_or(0)
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

            // BT.601 で RGB から YCbCr へ変換する
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
fn psnr_y(original: &[u8], decoded_surface: &Surface, width: usize, height: usize) -> f64 {
    let decoded_y = read_y_plane_from_surface(decoded_surface, width, height);
    let y_size = width * height;
    let mut mse_sum: f64 = 0.0;
    for i in 0..y_size {
        let diff = original[i] as f64 - decoded_y[i] as f64;
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
/// 各 EncodedFrame のデータはコールバックで蓄積される。
fn encode(context: &Context, config: EncoderConfig, frames: &[Vec<u8>]) -> Vec<EncodedFrame<()>> {
    encode_with_user_data(context, config, frames, |_| ())
}

/// 各入力フレームのインデックスからユーザーデータを作り、エンコードして出力を返す
///
/// B フレームを有効にすると出力順が入力順と異なるため、どの入力に対応する出力かを
/// `user_data` で識別するために使う。
fn encode_with_user_data<U, F>(
    context: &Context,
    config: EncoderConfig,
    frames: &[Vec<u8>],
    mut user_data: F,
) -> Vec<EncodedFrame<U>>
where
    U: Send + std::fmt::Debug + 'static,
    F: FnMut(usize) -> U,
{
    let width = config.width as usize;
    let height = config.height as usize;
    let format = config.frame_format;

    let result = Arc::new(Mutex::new(Vec::new()));
    let r = result.clone();
    let mut encoder = Encoder::new(
        context.clone(),
        config,
        FnEncodeHandler::<U>::new(move |result| {
            r.lock()
                .expect("エンコード結果を格納する Mutex のロックに失敗")
                .push(result.expect("エンコード結果の取得に失敗"));
        }),
    )
    .expect("エンコーダーの作成に失敗");
    let options = EncodeOptions {
        frame_type: frame_type::UNKNOWN,
    };

    for (index, frame) in frames.iter().enumerate() {
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_frame_to_surface(&surface, frame, format, width, height);
        encoder
            .encode(surface, &options, user_data(index))
            .expect("エンコードに失敗");
    }
    encoder.finish().expect("finish に失敗");
    drop(encoder);

    Arc::try_unwrap(result)
        .expect("Arc の参照が残っている")
        .into_inner()
        .expect("エンコード結果を格納する Mutex がポイズンされている")
}

/// エンコード結果をフレーム単位でデコードし、エンコード時の user_data を引き継ぐ
fn decode<U>(
    context: &Context,
    decoder_codec: DecoderCodec,
    encoded_frames: Vec<EncodedFrame<U>>,
) -> Vec<shiguredo_amf::DecodedFrame<U>>
where
    U: Send + std::fmt::Debug + 'static,
{
    let config = DecoderConfig {
        codec: decoder_codec,
    };
    let result = Arc::new(Mutex::new(Vec::new()));
    let r = result.clone();
    let mut decoder = Decoder::new(
        context.clone(),
        config,
        FnDecodeHandler::<U>::new(move |result| {
            r.lock()
                .expect("デコード結果を格納する Mutex のロックに失敗")
                .push(result.expect("デコード結果の取得に失敗"));
        }),
    )
    .expect("デコーダーの作成に失敗");

    // 1 フレーム分のビットストリームを格納した Buffer と、エンコード時の user_data を
    // デコーダーへ渡す。
    for encoded in encoded_frames {
        let (buffer, user_data) = encoded.into_parts();
        decoder.decode(buffer, user_data).expect("デコードに失敗");
    }

    decoder.finish().expect("finish に失敗");
    drop(decoder);

    Arc::try_unwrap(result)
        .expect("Arc の参照が残っている")
        .into_inner()
        .expect("デコード結果を格納する Mutex がポイズンされている")
}

/// フレームごとに指定したフレームタイプでエンコードし、出力を返す
///
/// `frame_types` の長さだけフレームを投入する。
fn encode_with_frame_types(
    context: &Context,
    config: EncoderConfig,
    frame_types: &[u16],
) -> Vec<EncodedFrame<()>> {
    let width = config.width as usize;
    let height = config.height as usize;
    let format = config.frame_format;

    let result = Arc::new(Mutex::new(Vec::new()));
    let r = result.clone();
    let mut encoder = Encoder::new(
        context.clone(),
        config,
        FnEncodeHandler::<()>::new(move |result| {
            r.lock()
                .expect("エンコード結果を格納する Mutex のロックに失敗")
                .push(result.expect("エンコード結果の取得に失敗"));
        }),
    )
    .expect("エンコーダーの作成に失敗");

    for (i, &frame_type) in frame_types.iter().enumerate() {
        let frame = generate_dummy_nv12(width, height, i);
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_frame_to_surface(&surface, &frame, format, width, height);
        encoder
            .encode(surface, &EncodeOptions { frame_type }, ())
            .expect("エンコードに失敗");
    }
    encoder.finish().expect("finish に失敗");
    drop(encoder);

    Arc::try_unwrap(result)
        .expect("Arc の参照が残っている")
        .into_inner()
        .expect("エンコード結果を格納する Mutex がポイズンされている")
}

/// 強制キーフレームを含むシーケンスをエンコードする
fn encode_with_forced_keyframe(
    context: &Context,
    config: EncoderConfig,
    force_at: usize,
    num_frames: usize,
) -> Vec<EncodedFrame<()>> {
    let mut frame_types = vec![frame_type::UNKNOWN; num_frames];
    frame_types[force_at] = frame_type::IDR | frame_type::I | frame_type::REF;
    encode_with_frame_types(context, config, &frame_types)
}

/// n 回目の IDR フレームのインデックスを返す（n は 1 始まり）
fn nth_idr_frame_index(encoded_frames: &[EncodedFrame<()>], nth: usize) -> usize {
    let mut count = 0usize;
    for (i, frame) in encoded_frames.iter().enumerate() {
        if frame.picture_type() == PictureType::Idr {
            count += 1;
            if count == nth {
                return i;
            }
        }
    }
    panic!("少なくとも {nth} 個の IDR フレームが必要だが {count} 個しかない");
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

/// エンコード→デコードのラウンドトリップを検証するヘルパー
fn roundtrip(
    encoder_config: EncoderConfig,
    decoder_codec: DecoderCodec,
    input_frames: &[Vec<u8>],
) -> Vec<shiguredo_amf::DecodedFrame<()>> {
    let width = encoder_config.width as usize;
    let height = encoder_config.height as usize;
    let num_frames = input_frames.len();

    // エンコードとデコードで同じ Context を共有する
    let context = create_context();
    let encoded_frames = encode(&context, encoder_config, input_frames);

    assert!(
        !encoded_frames.is_empty(),
        "エンコード済みフレームが生成されなかった"
    );
    for (i, frame) in encoded_frames.iter().enumerate() {
        assert!(
            frame.buffer().get_size() > 0,
            "エンコード済みフレーム {i} のバッファが空"
        );
    }

    let decoded_frames = decode(&context, decoder_codec, encoded_frames);

    assert_eq!(
        decoded_frames.len(),
        num_frames,
        "デコードされたフレーム数 {} が期待値 {num_frames} と一致しない",
        decoded_frames.len()
    );
    for (i, frame) in decoded_frames.iter().enumerate() {
        assert_eq!(
            surface_width(frame.surface()),
            width,
            "デコード済みフレーム {i} の幅が一致しない"
        );
        assert_eq!(
            surface_height(frame.surface()),
            height,
            "デコード済みフレーム {i} の高さが一致しない"
        );
        assert!(
            frame.surface().get_plane(PlaneType::Y).is_ok(),
            "デコード済みフレーム {i} の surface が無効"
        );
    }

    decoded_frames
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

    let decoded_frames = roundtrip(encoder_config, decoder_codec, &input_frames);

    for (i, decoded) in decoded_frames.iter().enumerate() {
        let psnr = psnr_y(&colorbar, decoded.surface(), width, height);
        assert!(
            psnr >= min_psnr_db,
            "フレーム {i}: PSNR {psnr:.1} dB が下限 {min_psnr_db} dB 未満"
        );
    }
}

// --- H.264 のテスト ---

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

/// H.264 で B フレームを有効にしたとき、全フレームが 1 回ずつ出力され、
/// user_data が対応する入力フレームに付くことを確認する
///
/// B フレームに対応していない GPU ではテストをスキップする。
#[test]
fn test_roundtrip_h264_b_pic_pattern_user_data() {
    let supports_b_frames = supported_codecs()
        .expect("コーデック対応状況の取得に失敗")
        .into_iter()
        .find(|info| info.codec == VideoCodecType::H264)
        .and_then(|info| info.encoding)
        .and_then(|encoding| encoding.supports_frame_reordering)
        .unwrap_or(false);
    if !supports_b_frames {
        eprintln!("H.264 の B フレームに対応していないためテストをスキップする");
        return;
    }

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
    // 1 GOP に 2 枚の B フレームを含める
    config.b_pic_pattern = Some(2);

    let num_frames = 20;
    let width = config.width as usize;
    let height = config.height as usize;
    let input_frames: Vec<Vec<u8>> = (0..num_frames)
        .map(|index| generate_dummy_nv12(width, height, index))
        .collect();

    let context = create_context();
    let encoded_frames = encode_with_user_data(&context, config, &input_frames, |index| index);

    assert_eq!(
        encoded_frames.len(),
        num_frames,
        "入力したフレーム数と出力フレーム数が一致しない"
    );
    let b_count = encoded_frames
        .iter()
        .filter(|frame| frame.picture_type() == PictureType::B)
        .count();
    assert!(b_count >= 1, "B フレームが 1 枚も出力されていない");

    // 各 user_data が 1 回ずつ出力されることを確認する。
    let mut counts = vec![0usize; num_frames];
    for frame in &encoded_frames {
        let index = *frame.user_data();
        assert!(index < num_frames, "user_data {index} が範囲外");
        counts[index] += 1;
    }
    for (index, count) in counts.iter().enumerate() {
        assert_eq!(
            *count, 1,
            "user_data {index} のフレームが {count} 回出力された"
        );
    }

    // デコード結果の内容と user_data を突き合わせ、対応付けが正しいことを確認する。
    let decoded_frames = decode(&context, DecoderCodec::H264, encoded_frames);
    assert_eq!(
        decoded_frames.len(),
        num_frames,
        "デコードされたフレーム数が入力したフレーム数と一致しない"
    );
    for frame in &decoded_frames {
        let index = *frame.user_data();
        let own_psnr = psnr_y(&input_frames[index], frame.surface(), width, height);
        for (other, other_frame) in input_frames.iter().enumerate() {
            if other == index {
                continue;
            }
            let other_psnr = psnr_y(other_frame, frame.surface(), width, height);
            assert!(
                own_psnr > other_psnr + 3.0,
                "user_data {index} のフレームが入力 {other} に近い \
                 (一致する入力との PSNR {own_psnr:.1} dB, 入力 {other} との PSNR {other_psnr:.1} dB)"
            );
        }
    }
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
    let format = config.frame_format;

    let context = create_context();
    let result = Arc::new(Mutex::new(Vec::new()));
    let r = result.clone();
    let mut encoder = Encoder::new(
        context.clone(),
        config,
        FnEncodeHandler::<()>::new(move |result| {
            r.lock()
                .expect("エンコード結果を格納する Mutex のロックに失敗")
                .push(result.expect("エンコード結果の取得に失敗"));
        }),
    )
    .expect("エンコーダーの作成に失敗");

    for i in 0..15 {
        let frame = generate_dummy_nv12(width, height, i);
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_frame_to_surface(&surface, &frame, format, width, height);

        let options = if i == 10 {
            EncodeOptions {
                frame_type: frame_type::IDR | frame_type::I,
            }
        } else {
            EncodeOptions {
                frame_type: frame_type::UNKNOWN,
            }
        };
        encoder
            .encode(surface, &options, ())
            .expect("エンコードに失敗");
    }
    encoder.finish().expect("finish に失敗");

    // コールバックで蓄積されたフレームを取り出す
    drop(encoder);
    let encoded_frames = Arc::try_unwrap(result)
        .expect("Arc の参照が残っている")
        .into_inner()
        .expect("エンコード結果を格納する Mutex がポイズンされている");

    let idr_count = encoded_frames
        .iter()
        .filter(|f| f.picture_type() == PictureType::Idr)
        .count();
    assert!(
        idr_count >= 2,
        "少なくとも 2 個の IDR フレームが必要だが {idr_count} 個しかない"
    );

    // フレーム単位でデコードして復号できることを確認する
    let decoded_frames = decode(&context, DecoderCodec::H264, encoded_frames);
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

    let context = create_context();
    let encoded_frames = encode_with_forced_keyframe(&context, config, 10, 15);
    let forced_idr_index = nth_idr_frame_index(&encoded_frames, 2);
    let forced_idr = &encoded_frames[forced_idr_index];

    assert_eq!(forced_idr.picture_type(), PictureType::Idr);
    assert!(
        h264_contains_sps_pps(buffer_to_slice(forced_idr.buffer())),
        "強制 H.264 IDR フレームに SPS と PPS が含まれていること"
    );
}

/// H.264 で I と P を強制すると出力のピクチャタイプが指定どおりになることを確認する
#[test]
fn test_force_picture_type_h264_i_and_p() {
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

    let mut frame_types = vec![frame_type::UNKNOWN; 10];
    frame_types[5] = frame_type::I;
    frame_types[7] = frame_type::P;

    let encoded_frames = encode_with_frame_types(&create_context(), config, &frame_types);
    assert_eq!(
        encoded_frames.len(),
        frame_types.len(),
        "入力したフレーム数と出力フレーム数が一致すること"
    );
    assert_eq!(
        encoded_frames[5].picture_type(),
        PictureType::I,
        "I を強制したフレームのピクチャタイプが I であること"
    );
    assert_eq!(
        encoded_frames[7].picture_type(),
        PictureType::P,
        "P を強制したフレームのピクチャタイプが P であること"
    );
}

/// 複数のフレームタイプを指定した場合の優先順位と、未対応フラグが無視されることを確認する
#[test]
fn test_force_picture_type_priority_and_ignored_flags() {
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

    let mut frame_types = vec![frame_type::UNKNOWN; 12];
    // IDR が最優先される
    frame_types[2] = frame_type::IDR | frame_type::I | frame_type::P;
    // IDR がなければ I が優先される
    frame_types[4] = frame_type::I | frame_type::P;
    // I がなければ P が使われる
    frame_types[6] = frame_type::P | frame_type::B | frame_type::REF;
    // 強制に使えるフラグがない場合は何も強制しない
    frame_types[8] = frame_type::B | frame_type::REF | 0x8000;

    let encoded_frames = encode_with_frame_types(&create_context(), config, &frame_types);
    assert_eq!(
        encoded_frames.len(),
        frame_types.len(),
        "入力したフレーム数と出力フレーム数が一致すること"
    );
    assert_eq!(
        encoded_frames[2].picture_type(),
        PictureType::Idr,
        "IDR が指定された場合は IDR が優先されること"
    );
    assert_eq!(
        encoded_frames[4].picture_type(),
        PictureType::I,
        "IDR がなく I が指定された場合は I が優先されること"
    );
    assert_eq!(
        encoded_frames[6].picture_type(),
        PictureType::P,
        "I がなく P が指定された場合は P が使われること"
    );
    assert_eq!(
        encoded_frames[8].picture_type(),
        PictureType::P,
        "強制に使えるフラグがない場合は自然なピクチャタイプになること"
    );
}

// --- H.265 のテスト ---

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

    let context = create_context();
    let encoded_frames = encode_with_forced_keyframe(&context, config, 10, 15);
    let forced_idr_index = nth_idr_frame_index(&encoded_frames, 2);
    let forced_idr = &encoded_frames[forced_idr_index];

    assert_eq!(forced_idr.picture_type(), PictureType::Idr);
    assert!(
        hevc_contains_vps_sps_pps(buffer_to_slice(forced_idr.buffer())),
        "強制 HEVC IDR フレームに VPS, SPS, PPS が含まれていること"
    );
}

/// HEVC で I と P を強制すると出力のピクチャタイプが指定どおりになることを確認する
#[test]
fn test_force_picture_type_hevc_i_and_p() {
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

    let mut frame_types = vec![frame_type::UNKNOWN; 10];
    frame_types[5] = frame_type::I;
    frame_types[7] = frame_type::P;

    let encoded_frames = encode_with_frame_types(&create_context(), config, &frame_types);
    assert_eq!(
        encoded_frames.len(),
        frame_types.len(),
        "入力したフレーム数と出力フレーム数が一致すること"
    );
    assert_eq!(
        encoded_frames[5].picture_type(),
        PictureType::I,
        "I を強制したフレームのピクチャタイプが I であること"
    );
    assert_eq!(
        encoded_frames[7].picture_type(),
        PictureType::P,
        "P を強制したフレームのピクチャタイプが P であること"
    );
}

// --- AV1 のテスト ---

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

/// AV1 で先頭フレームがキーフレーム、後続の INTER フレームが P と通知されることを確認する
#[test]
fn test_av1_picture_types() {
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

    let frames: Vec<Vec<u8>> = (0..5).map(|i| generate_dummy_nv12(320, 240, i)).collect();
    let encoded_frames = encode(&create_context(), config, &frames);
    assert_eq!(
        encoded_frames.len(),
        frames.len(),
        "入力したフレーム数と出力フレーム数が一致すること"
    );

    let first_frame = encoded_frames.first().expect("先頭フレームの取得に失敗");
    assert_eq!(
        first_frame.picture_type(),
        PictureType::Idr,
        "先頭フレームはキーフレームなので Idr になること"
    );
    for (i, frame) in encoded_frames.iter().enumerate().skip(1) {
        assert_eq!(
            frame.picture_type(),
            PictureType::P,
            "フレーム {i} は INTER なので P になること"
        );
    }
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

    let context = create_context();
    let encoded_frames = encode_with_forced_keyframe(&context, config, 10, 15);
    let forced_key_index = nth_idr_frame_index(&encoded_frames, 2);
    let forced_key = &encoded_frames[forced_key_index];

    assert_eq!(forced_key.picture_type(), PictureType::Idr);
    assert!(
        av1_contains_sequence_header_obu(buffer_to_slice(forced_key.buffer())),
        "強制 AV1 キーフレームにシーケンスヘッダー OBU が含まれていること"
    );
}

/// AV1 で I を強制すると出力のピクチャタイプが I になることを確認する
#[test]
fn test_force_picture_type_av1_i() {
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

    let mut frame_types = vec![frame_type::UNKNOWN; 10];
    frame_types[5] = frame_type::I;

    let encoded_frames = encode_with_frame_types(&create_context(), config, &frame_types);
    assert_eq!(
        encoded_frames.len(),
        frame_types.len(),
        "入力したフレーム数と出力フレーム数が一致すること"
    );
    assert_eq!(
        encoded_frames[5].picture_type(),
        PictureType::I,
        "I を強制したフレームのピクチャタイプが I であること"
    );
}

/// AV1 では P が無視され、I が指定されていれば I が強制されることを確認する
#[test]
fn test_force_picture_type_av1_ignores_p() {
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

    let mut frame_types = vec![frame_type::UNKNOWN; 10];
    frame_types[3] = frame_type::P;
    frame_types[5] = frame_type::P | frame_type::I;

    let encoded_frames = encode_with_frame_types(&create_context(), config, &frame_types);
    assert_eq!(
        encoded_frames.len(),
        frame_types.len(),
        "入力したフレーム数と出力フレーム数が一致すること"
    );
    assert_eq!(
        encoded_frames[3].picture_type(),
        PictureType::P,
        "AV1 で P を指定しても強制されず自然なピクチャタイプになること"
    );
    assert_eq!(
        encoded_frames[5].picture_type(),
        PictureType::I,
        "P と I を指定した場合は I が優先されること"
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
    let context = create_context();
    let mut encoder = Encoder::new(context.clone(), config, FnEncodeHandler::<()>::new(|_| {}))
        .expect("エンコーダーの作成に失敗");

    let err = encoder
        .reconfigure(ReconfigureParams {
            framerate_num: Some(0),
            framerate_den: Some(1),
            ..ReconfigureParams::default()
        })
        .expect_err("reconfigure はゼロフレームレートでエラーになること");
    assert!(
        matches!(&err, Error::InvalidArgument(_)),
        "ゼロフレームレートの reconfigure が InvalidArgument ではない: {err}"
    );
    assert!(
        err.to_string().contains("invalid framerate"),
        "想定外のエラー: {err}"
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
    let format = config.frame_format;

    let context = create_context();
    let result = Arc::new(Mutex::new(Vec::new()));
    let r = result.clone();
    let mut encoder = Encoder::new(
        context.clone(),
        config,
        FnEncodeHandler::<()>::new(move |result| {
            r.lock()
                .expect("エンコード結果を格納する Mutex のロックに失敗")
                .push(result.expect("エンコード結果の取得に失敗"));
        }),
    )
    .expect("エンコーダーの作成に失敗");
    let options = EncodeOptions {
        frame_type: frame_type::UNKNOWN,
    };

    for i in 0..5 {
        let frame = generate_dummy_nv12(320, 240, i);
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_frame_to_surface(&surface, &frame, format, 320, 240);
        encoder
            .encode(surface, &options, ())
            .expect("エンコードに失敗");
    }

    encoder
        .reconfigure(ReconfigureParams {
            framerate_num: Some(15),
            framerate_den: Some(1),
            target_kbps: Some(1500),
            ..ReconfigureParams::default()
        })
        .expect("reconfigure に失敗");

    for i in 5..10 {
        let frame = generate_dummy_nv12(320, 240, i);
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_frame_to_surface(&surface, &frame, format, 320, 240);
        encoder
            .encode(surface, &options, ())
            .expect("エンコードに失敗");
    }

    encoder.finish().expect("finish に失敗");
    drop(encoder);
    let encoded_frames = Arc::try_unwrap(result)
        .expect("Arc の参照が残っている")
        .into_inner()
        .expect("エンコード結果を格納する Mutex がポイズンされている");
    assert!(
        !encoded_frames.is_empty(),
        "エンコード済みフレームが生成されなかった"
    );

    let decoded = decode(&context, DecoderCodec::H264, encoded_frames);
    assert_eq!(decoded.len(), 10);
}

/// HEVC で未対応の qpb/gop_pic_size を reconfigure に指定するとエラーになることを確認する
#[test]
fn test_reconfigure_hevc_rejects_qpb_and_gop() {
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

    let mut encoder = Encoder::new(create_context(), config, FnEncodeHandler::<()>::new(|_| {}))
        .expect("エンコーダーの作成に失敗");

    let err = encoder
        .reconfigure(ReconfigureParams {
            qpb: Some(28),
            ..ReconfigureParams::default()
        })
        .expect_err("HEVC の qpb を再設定できてしまった");
    assert!(
        matches!(&err, Error::InvalidArgument(_)),
        "Encoder::reconfigure の引数エラーであるべき: {err}"
    );
    assert!(
        err.to_string().contains("qpb is not supported for HEVC"),
        "想定したエラーメッセージではない: {err}"
    );

    let err = encoder
        .reconfigure(ReconfigureParams {
            gop_pic_size: Some(20),
            ..ReconfigureParams::default()
        })
        .expect_err("HEVC の gop_pic_size を再設定できてしまった");
    assert!(
        matches!(&err, Error::InvalidArgument(_)),
        "Encoder::reconfigure の引数エラーであるべき: {err}"
    );
    assert!(
        err.to_string()
            .contains("gop_pic_size cannot be changed for HEVC"),
        "想定したエラーメッセージではない: {err}"
    );

    // 未対応項目を含まない再設定は成功する
    encoder
        .reconfigure(ReconfigureParams {
            framerate_num: Some(24),
            framerate_den: Some(1),
            target_kbps: Some(1200),
            qpi: Some(24),
            qpp: Some(26),
            ..ReconfigureParams::default()
        })
        .expect("対応項目だけの reconfigure に失敗");
    encoder.finish().expect("finish に失敗");
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
    let format = config.frame_format;

    let context = create_context();
    let result = Arc::new(Mutex::new(Vec::new()));
    let r = result.clone();
    let mut encoder = Encoder::new(
        context.clone(),
        config,
        FnEncodeHandler::<()>::new(move |result| {
            r.lock()
                .expect("エンコード結果を格納する Mutex のロックに失敗")
                .push(result.expect("エンコード結果の取得に失敗"));
        }),
    )
    .expect("エンコーダーの作成に失敗");
    let options = EncodeOptions {
        frame_type: frame_type::UNKNOWN,
    };

    for i in 0..4 {
        let frame = generate_dummy_nv12(320, 240, i);
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_frame_to_surface(&surface, &frame, format, 320, 240);
        encoder
            .encode(surface, &options, ())
            .expect("エンコードに失敗");
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
        .expect("reconfigure に失敗");

    for i in 4..8 {
        let frame = generate_dummy_nv12(320, 240, i);
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_frame_to_surface(&surface, &frame, format, 320, 240);
        encoder
            .encode(surface, &options, ())
            .expect("エンコードに失敗");
    }

    encoder.finish().expect("finish に失敗");
    drop(encoder);
    let encoded_frames = Arc::try_unwrap(result)
        .expect("Arc の参照が残っている")
        .into_inner()
        .expect("エンコード結果を格納する Mutex がポイズンされている");
    assert!(
        !encoded_frames.is_empty(),
        "エンコード済みフレームが生成されなかった"
    );

    let decoded = decode(&context, DecoderCodec::Av1, encoded_frames);
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
            data[offset] = v; // 青 (B)
            data[offset + 1] = v; // 緑 (G)
            data[offset + 2] = v; // 赤 (R)
            data[offset + 3] = 255; // アルファ (A)
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
            data[offset] = 255; // アルファ (A)
            data[offset + 1] = v; // 赤 (R)
            data[offset + 2] = v; // 緑 (G)
            data[offset + 3] = v; // 青 (B)
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
            data[offset] = v; // 赤 (R)
            data[offset + 1] = v; // 緑 (G)
            data[offset + 2] = v; // 青 (B)
            data[offset + 3] = 255; // アルファ (A)
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
            data[offset] = v; // 輝度 Y0
            data[offset + 1] = 128; // 色差 U
            data[offset + 2] = v; // 輝度 Y1
            data[offset + 3] = 128; // 色差 V
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
        .expect("フレームサイズがオーバーフロー");
    for (i, frame) in input_frames.iter().enumerate() {
        assert_eq!(
            frame.len(),
            expected_size,
            "フレーム {i}: サイズ {len} != 期待値 {expected_size} ({format:?})",
            len = frame.len()
        );
    }

    let decoded_frames = roundtrip(config, decoder_codec, &input_frames);

    assert_eq!(
        decoded_frames.len(),
        num_frames,
        "{format:?}: デコードされたフレーム数 {len} が期待値 {num_frames} と一致しない",
        len = decoded_frames.len()
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

    let decoded_frames = roundtrip(config, DecoderCodec::H264, &input_frames);

    for (i, decoded) in decoded_frames.iter().enumerate() {
        let psnr = psnr_y(
            &colorbar,
            decoded.surface(),
            width as usize,
            height as usize,
        );
        assert!(
            psnr >= 25.0,
            "フレーム {i}: PSNR {psnr:.1} dB が下限 25.0 dB 未満"
        );
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

    let decoded_frames = roundtrip(config, DecoderCodec::Hevc, &input_frames);

    for (i, decoded) in decoded_frames.iter().enumerate() {
        let psnr = psnr_y(
            &colorbar,
            decoded.surface(),
            width as usize,
            height as usize,
        );
        assert!(
            psnr >= 25.0,
            "フレーム {i}: PSNR {psnr:.1} dB が下限 25.0 dB 未満"
        );
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

    let decoded_frames = roundtrip(config, DecoderCodec::Av1, &input_frames);

    for (i, decoded) in decoded_frames.iter().enumerate() {
        let psnr = psnr_y(
            &colorbar,
            decoded.surface(),
            width as usize,
            height as usize,
        );
        assert!(
            psnr >= 25.0,
            "フレーム {i}: PSNR {psnr:.1} dB が下限 25.0 dB 未満"
        );
    }
}

// ---------------------------------------------------------------------------
// コールバックに渡した T が正しい順序で届くことのテスト
// ---------------------------------------------------------------------------

/// encode() に渡した T がコールバックに正しい順序で届くことを確認する
#[test]
fn test_user_data_delivered_in_submit_order() {
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

    let context = create_context();
    let received = Arc::new(Mutex::new(Vec::new()));
    let r = received.clone();

    let mut encoder = Encoder::new(
        context.clone(),
        config,
        FnEncodeHandler::<usize>::new(move |result| {
            r.lock()
                .expect("エンコード結果を格納する Mutex のロックに失敗")
                .push(result.expect("エンコード結果の取得に失敗").into_parts().1);
        }),
    )
    .expect("エンコーダーの作成に失敗");

    let width: usize = 320;
    let height: usize = 240;
    let num_frames: usize = 10;
    let options = EncodeOptions::default();

    for i in 0..num_frames {
        let data = generate_dummy_nv12(width, height, i);
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_nv12_to_surface(&surface, &data, width, height);
        encoder
            .encode(surface, &options, i)
            .expect("エンコードに失敗");
    }
    encoder.finish().expect("finish に失敗");
    drop(encoder);

    let tags = Arc::try_unwrap(received)
        .expect("Arc の参照が残っている")
        .into_inner()
        .expect("エンコード結果を格納する Mutex がポイズンされている");
    assert_eq!(tags.len(), num_frames);
    assert_eq!(tags, (0..num_frames).collect::<Vec<_>>());
}

// ---------------------------------------------------------------------------
// コンポーネント投入時のハンドル一意性のテスト
// ---------------------------------------------------------------------------

/// Surface のクローンが残っている場合はエンコードが拒否されることを確認する
#[test]
fn test_encode_rejects_surface_with_cloned_handle() {
    let context = create_context();
    let mut encoder = Encoder::new(
        context.clone(),
        h264_cbr_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");

    let width = 320;
    let height = 240;
    let data = generate_dummy_nv12(width, height, 0);
    let options = EncodeOptions::default();

    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    copy_nv12_to_surface(&surface, &data, width, height);
    let cloned = surface.clone();

    let result = encoder.encode(surface, &options, ());
    let err = result.expect_err("クローンが残っている Surface のエンコードは失敗するはず");
    assert!(
        matches!(&err, Error::InvalidState(_)),
        "共有 Surface のエンコードが InvalidState ではない: {err}"
    );
    assert!(
        err.to_string().contains("shared with other handles"),
        "共有ハンドル拒否のエラーではない: {err}"
    );
    drop(cloned);

    // クローンを破棄した後の Surface は投入できる
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    copy_nv12_to_surface(&surface, &data, width, height);
    encoder
        .encode(surface, &options, ())
        .expect("クローン破棄後のエンコードに失敗");
    encoder.finish().expect("finish に失敗");
}

/// PropertyStorage を保持したままの Surface のエンコードが拒否されることを確認する
#[test]
fn test_encode_rejects_surface_with_live_property_storage() {
    let context = create_context();
    let mut encoder = Encoder::new(
        context.clone(),
        h264_cbr_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");

    let width = 320;
    let height = 240;
    let data = generate_dummy_nv12(width, height, 0);
    let options = EncodeOptions::default();

    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    copy_nv12_to_surface(&surface, &data, width, height);
    let property_storage = surface
        .property_storage()
        .expect("PropertyStorage の取得に失敗");

    let result = encoder.encode(surface, &options, ());
    let err = result.expect_err("PropertyStorage が残っている Surface のエンコードは失敗するはず");
    assert!(
        matches!(&err, Error::InvalidState(_)),
        "PropertyStorage が残る Surface のエンコードが InvalidState ではない: {err}"
    );
    assert!(
        err.to_string().contains("shared with other handles"),
        "共有ハンドル拒否のエラーではない: {err}"
    );
    drop(property_storage);
}

/// Buffer のクローンが残っている場合はデコードが拒否されることを確認する
#[test]
fn test_decode_rejects_buffer_with_cloned_handle() {
    let context = create_context();
    let encoded_frames = encode(
        &context,
        h264_cbr_test_encoder_config(),
        &[generate_dummy_nv12(320, 240, 0)],
    );
    let mut encoded = encoded_frames.into_iter();
    let first = encoded.next().expect("エンコード結果が空");
    let cloned = first.buffer().clone();

    let mut decoder = Decoder::new(
        context.clone(),
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<()>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");

    let result = decoder.decode(first.into_parts().0, ());
    let err = result.expect_err("クローンが残っている Buffer のデコードは失敗するはず");
    assert!(
        matches!(&err, Error::InvalidState(_)),
        "共有 Buffer のデコードが InvalidState ではない: {err}"
    );
    assert!(
        err.to_string().contains("shared with other handles"),
        "共有ハンドル拒否のエラーではない: {err}"
    );
    drop(cloned);
}

// ---------------------------------------------------------------------------
// Context ごとの独立性と Context 間の受け渡し拒否のテスト
// ---------------------------------------------------------------------------

/// デコーダーが出力した Surface を同じ Context の別の Encoder にそのまま投入できることを確認する
#[test]
fn test_decoded_surface_can_be_reencoded() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;
    const NUM_FRAMES: usize = 3;

    let context = create_context();
    let input_frames: Vec<Vec<u8>> = (0..NUM_FRAMES)
        .map(|i| generate_dummy_nv12(WIDTH, HEIGHT, i))
        .collect();
    let encoded_frames = encode(&context, h264_cbr_test_encoder_config(), &input_frames);
    let decoded_frames = decode(&context, DecoderCodec::H264, encoded_frames);
    assert_eq!(
        decoded_frames.len(),
        NUM_FRAMES,
        "デコードされたフレーム数が期待値と一致しない"
    );

    // デコード結果の Surface を、デコーダーとは別の Encoder にそのまま投入する
    let reencoded = Arc::new(Mutex::new(Vec::new()));
    let r = Arc::clone(&reencoded);
    let mut encoder = Encoder::new(
        context.clone(),
        h264_cbr_test_encoder_config(),
        FnEncodeHandler::<()>::new(move |result| {
            r.lock()
                .expect("再エンコード結果の記録用 Mutex がポイズンされた")
                .push(result.expect("再エンコード結果の受信に失敗"));
        }),
    )
    .expect("エンコーダーの作成に失敗");

    for frame in decoded_frames {
        let (surface, _) = frame.into_parts();
        encoder
            .encode(surface, &EncodeOptions::default(), ())
            .expect("デコード済み Surface の再エンコードに失敗");
    }
    encoder.finish().expect("finish に失敗");
    drop(encoder);

    let reencoded = Arc::try_unwrap(reencoded)
        .expect("再エンコード結果の Arc の参照が残っている")
        .into_inner()
        .expect("再エンコード結果の Mutex がポイズンされた");
    assert_eq!(
        reencoded.len(),
        NUM_FRAMES,
        "再エンコードされたフレーム数が期待値と一致しない"
    );
    for (i, frame) in reencoded.iter().enumerate() {
        assert!(
            frame.buffer().get_size() > 0,
            "再エンコード済みフレーム {i} のバッファが空"
        );
    }
}

/// 別の Context から確保した Surface はエンコードできないことを確認する
#[test]
fn test_encode_rejects_surface_from_other_context() {
    let encoder_context = create_context();
    let surface_context = create_context();
    let mut encoder = Encoder::new(
        encoder_context.clone(),
        h264_cbr_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");

    let surface = surface_context
        .alloc_surface(
            MemoryType::Host,
            SurfaceFormat::from(FrameFormat::Nv12),
            320,
            240,
        )
        .expect("別 Context の Surface の確保に失敗");

    let result = encoder.encode(surface, &EncodeOptions::default(), ());
    let err = result.expect_err("別 Context の Surface のエンコードは失敗するはず");
    assert!(
        matches!(&err, Error::InvalidArgument(_)),
        "別 Context の Surface のエンコードが InvalidArgument ではない: {err}"
    );
    assert!(
        err.to_string().contains("does not belong"),
        "Context 不一致のエラーではない: {err}"
    );
}

/// 別の Context から確保した Buffer はデコードできないことを確認する
#[test]
fn test_decode_rejects_buffer_from_other_context() {
    let encoder_context = create_context();
    let decoder_context = create_context();

    let encoded_frames = encode(
        &encoder_context,
        h264_cbr_test_encoder_config(),
        &[generate_dummy_nv12(320, 240, 0)],
    );
    let mut encoded = encoded_frames.into_iter();
    let first = encoded.next().expect("エンコード結果が空");

    let mut decoder = Decoder::new(
        decoder_context,
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<()>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");

    let result = decoder.decode(first.into_parts().0, ());
    let err = result.expect_err("別 Context の Buffer のデコードは失敗するはず");
    assert!(
        matches!(&err, Error::InvalidArgument(_)),
        "別 Context の Buffer のデコードが InvalidArgument ではない: {err}"
    );
    assert!(
        err.to_string().contains("does not belong"),
        "Context 不一致のエラーではない: {err}"
    );
}

/// 異なる Context の Encoder が独立して動作することを確認する
#[test]
fn test_multiple_contexts_encode_independently() {
    let context_a = create_context();
    let context_b = create_context();
    let frame = generate_dummy_nv12(320, 240, 0);

    for context in [context_a, context_b] {
        let mut encoder = Encoder::new(
            context,
            h264_cbr_test_encoder_config(),
            FnEncodeHandler::<()>::new(|_| {}),
        )
        .expect("エンコーダーの作成に失敗");
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        copy_nv12_to_surface(&surface, &frame, 320, 240);
        encoder
            .encode(surface, &EncodeOptions::default(), ())
            .expect("エンコードに失敗");
        encoder.finish().expect("finish に失敗");
    }
}
