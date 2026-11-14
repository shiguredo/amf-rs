use std::sync::mpsc;

use shiguredo_amf::amf::Surface;
use shiguredo_amf::ffi::{AMF_PLANE_TYPE, AMF_RESULT};
use shiguredo_amf::{
    Av1EncoderConfig, Av1Profile, CodecConfig, EncodeOptions, Encoder, EncoderConfig, Error,
    FnEncodeHandler, FrameFormat, HevcEncoderConfig, HevcProfile, RateControlMode,
    ReconfigureParams,
};

pub mod helpers;

use helpers::create_context;

/// ReconfigureParams::default() が全項目未指定であることを確認する
#[test]
fn test_reconfigure_params_default_is_empty() {
    let params = ReconfigureParams::default();
    assert_eq!(params.framerate_num, None);
    assert_eq!(params.framerate_den, None);
    assert_eq!(params.target_kbps, None);
    assert_eq!(params.max_kbps, None);
    assert_eq!(params.qpi, None);
    assert_eq!(params.qpp, None);
    assert_eq!(params.qpb, None);
    assert_eq!(params.gop_pic_size, None);
}

/// テスト用の HEVC Main10 プロファイル設定を返す
fn hevc_main10() -> CodecConfig {
    CodecConfig::Hevc(HevcEncoderConfig {
        profile: Some(HevcProfile::Main10),
    })
}

/// テスト用の HEVC Main プロファイル設定を返す
fn hevc_main() -> CodecConfig {
    CodecConfig::Hevc(HevcEncoderConfig {
        profile: Some(HevcProfile::Main),
    })
}

/// テスト用の HEVC プロファイル未指定の設定を返す
fn hevc_default_profile() -> CodecConfig {
    CodecConfig::Hevc(HevcEncoderConfig { profile: None })
}

/// テスト用の AV1 Main プロファイル設定を返す
fn av1_main() -> CodecConfig {
    CodecConfig::Av1(Av1EncoderConfig {
        profile: Some(Av1Profile::Main),
    })
}

/// 指定したコーデックと入力フォーマットのエンコーダー設定を返す
fn test_encoder_config(codec: CodecConfig, frame_format: FrameFormat) -> EncoderConfig {
    let mut config = EncoderConfig::new(codec, 320, 240, frame_format, 30, 1, RateControlMode::Cbr);
    config.target_kbps = Some(1000);
    config
}

/// 指定した設定でエンコーダーを作成する
fn create_encoder(config: EncoderConfig) -> Result<Encoder<FnEncodeHandler<()>>, Error> {
    Encoder::new(create_context(), config, FnEncodeHandler::<()>::new(|_| {}))
}

/// P010 のフレームデータを Surface に書き込む
///
/// Y プレーンはフレーム番号に応じたグラデーション、UV プレーンは 0x8000 固定にする。
fn write_p010_surface(surface: &Surface, width: usize, height: usize, frame_index: usize) {
    let y_plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");
    let y_native = y_plane.get_native() as *mut u16;
    // get_hpitch() はバイト単位なので、16bit サンプルの要素数に変換する
    let y_pitch = y_plane.get_hpitch() as usize / 2;
    for row in 0..height {
        for col in 0..width {
            // 10bit サンプルを 16bit ワードの上位ビットに格納する
            let value = (((col + row + frame_index * 7) % 256) as u16) << 6;
            unsafe {
                *y_native.add(row * y_pitch + col) = value;
            }
        }
    }

    let uv_plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_UV)
        .expect("UV プレーンの取得に失敗");
    let uv_native = uv_plane.get_native() as *mut u16;
    let uv_pitch = uv_plane.get_hpitch() as usize / 2;
    for row in 0..height / 2 {
        for col in 0..width {
            // 10bit の無彩色 (512) を 16bit ワードの上位ビットに格納する
            unsafe {
                *uv_native.add(row * uv_pitch + col) = 512 << 6;
            }
        }
    }
}

/// HEVC の 10bit 入力には Main10 プロファイルが必要なことを確認する
///
/// AMF は Main プロファイルのまま 10bit をエンコードできてしまい、
/// Main プロファイルに適合しないストリームを生成するため、Encoder::new で拒否する。
#[test]
fn test_encoder_rejects_hevc_10bit_without_main10() {
    let cases = [
        ("HevcProfile::Main", "HevcProfile::Main with", hevc_main()),
        (
            "no profile",
            "the default HEVC profile (Main)",
            hevc_default_profile(),
        ),
    ];
    for frame_format in [FrameFormat::P010, FrameFormat::Y210, FrameFormat::Y410] {
        for (name, expected_profile, codec) in &cases {
            let err = create_encoder(test_encoder_config(codec.clone(), frame_format))
                .err()
                .expect("10bit 入力と Main プロファイルなのにエンコーダーを作成できた");
            assert_eq!(
                err.status(),
                None,
                "{name} {frame_format:?}: AMF のエラーではなく Encoder::new のエラーであるべき: {err}"
            );
            let message = err.to_string();
            assert!(
                message.contains("HEVC 10-bit input requires HevcProfile::Main10"),
                "{name} {frame_format:?}: 想定したエラーメッセージではない: {message}"
            );
            assert!(
                message.contains(*expected_profile),
                "{name} {frame_format:?}: プロファイルの説明が含まれていない: {message}"
            );
        }
    }
}

/// P010 入力で HEVC Main10 と AV1 のエンコーダーを初期化し、エンコードできることを確認する
#[test]
fn test_encoder_p010_10bit_encoding() {
    for (name, codec) in [("HEVC Main10", hevc_main10()), ("AV1 Main", av1_main())] {
        let (tx, rx) = mpsc::channel();
        let encoder = Encoder::new(
            create_context(),
            test_encoder_config(codec, FrameFormat::P010),
            FnEncodeHandler::<()>::new(move |result| {
                // ハンドラーはワーカースレッドから呼ばれるため、結果はチャネルで受け取る
                tx.send(result).expect("エンコード結果の送信に失敗");
            }),
        );
        let mut encoder = match encoder {
            Ok(encoder) => encoder,
            Err(e) => {
                // 10bit エンコードに対応していない GPU では AMF の Init が失敗する
                assert_eq!(
                    e.status(),
                    Some(AMF_RESULT::AMF_INVALID_FORMAT),
                    "{name}: P010 のエンコーダーを作成できず、想定していないエラーになった: {e}"
                );
                continue;
            }
        };

        let num_frames = 5;
        for frame_index in 0..num_frames {
            let surface = encoder
                .alloc_surface()
                .unwrap_or_else(|e| panic!("{name}: Surface の確保に失敗: {e}"));
            write_p010_surface(&surface, 320, 240, frame_index);
            encoder
                .encode(surface, &EncodeOptions::default(), ())
                .unwrap_or_else(|e| {
                    panic!("{name}: フレーム {frame_index} のエンコードに失敗: {e}")
                });
        }
        encoder
            .finish()
            .unwrap_or_else(|e| panic!("{name}: finish に失敗: {e}"));
        drop(encoder);

        let frames: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            frames.len(),
            num_frames,
            "{name}: エンコードされたフレーム数が一致しない"
        );
        for (frame_index, result) in frames.iter().enumerate() {
            match result {
                Ok(frame) => {
                    assert!(
                        frame.buffer().get_size() > 0,
                        "{name}: フレーム {frame_index} のビットストリームが空"
                    );
                }
                Err(e) => panic!("{name}: フレーム {frame_index} のエンコードに失敗: {e}"),
            }
        }
    }
}

/// Y210 / Y410 の初期化が GPU と AMF の実装に依存することを確認する
///
/// 対応していない GPU では AMF の Init が AMF_INVALID_FORMAT を返す。
#[test]
fn test_encoder_y210_y410_initialization() {
    for frame_format in [FrameFormat::Y210, FrameFormat::Y410] {
        for (name, codec) in [("HEVC Main10", hevc_main10()), ("AV1 Main", av1_main())] {
            match create_encoder(test_encoder_config(codec, frame_format)) {
                Ok(encoder) => {
                    // 対応している GPU では初期化に成功する
                    drop(encoder);
                }
                Err(e) => {
                    assert_eq!(
                        e.status(),
                        Some(AMF_RESULT::AMF_INVALID_FORMAT),
                        "{name} {frame_format:?}: 想定していないエラー: {e}"
                    );
                }
            }
        }
    }
}
