use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use shiguredo_amf::amf::Surface;
use shiguredo_amf::ffi::AMF_RESULT;
use shiguredo_amf::{
    Av1EncoderConfig, Av1Profile, CodecConfig, EncodeHandler, EncodeOptions, EncodedFrame, Encoder,
    EncoderConfig, Error, FnEncodeHandler, FrameFormat, H264EncoderConfig, H264Profile,
    HevcEncoderConfig, HevcProfile, PlaneType, RateControlMode, ReconfigureParams, VideoCodecType,
    supported_codecs,
};

pub mod helpers;

use helpers::{
    copy_nv12_to_surface, create_context, generate_dummy_nv12, h264_cbr_test_encoder_config,
};

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

/// 最初の通知でワーカーを停止させるハンドラー
///
/// 最初のエンコード結果を記録すると `first_seen_tx` で通知し、`release_rx` を
/// 受け取るまで [`EncodeHandler::on_encoded`] から戻らない。ワーカーが停止して
/// いる間に `encode` を呼ぶと、その入力はワーカーに処理されずチャネルに残る。
struct BlockingEncodeHandler {
    /// 最初の通知を送るチャネル
    first_seen_tx: mpsc::Sender<()>,
    /// 最初の通知の処理を再開するチャネル
    release_rx: mpsc::Receiver<()>,
    /// 通知された user_data の一覧
    notified: Arc<Mutex<Vec<usize>>>,
    /// 最初の通知の処理中か
    blocked: bool,
}

impl EncodeHandler for BlockingEncodeHandler {
    type UserData = usize;
    type Error = Error;

    fn on_encoded(&mut self, result: Result<EncodedFrame<usize>, Error>) {
        let frame = result.expect("エンコード結果がエラーになった");
        self.notified
            .lock()
            .expect("通知一覧のロックに失敗")
            .push(*frame.user_data());
        if !self.blocked {
            self.blocked = true;
            self.first_seen_tx.send(()).expect("最初の通知の送信に失敗");
            self.release_rx.recv().expect("解放の受信に失敗");
        }
    }
}

/// 入力を 1 つも投入せずに finish した場合、通知がなく完了することを確認する
#[test]
fn test_encoder_finish_without_input() {
    let (tx, rx) = mpsc::channel();
    let mut encoder = Encoder::new(
        create_context(),
        h264_cbr_test_encoder_config(),
        FnEncodeHandler::<()>::new(move |result| {
            tx.send(result).expect("通知の送信に失敗");
        }),
    )
    .expect("エンコーダーの作成に失敗");

    encoder.finish().expect("入力なしの finish に失敗");
    drop(encoder);

    assert!(
        rx.try_iter().next().is_none(),
        "入力なしの finish で通知があった"
    );
}

/// finish 後に encode を呼ぶと、AMF を呼ばずにエラーになることを確認する
#[test]
fn test_encode_after_finish_is_rejected() {
    let mut encoder =
        create_encoder(h264_cbr_test_encoder_config()).expect("エンコーダーの作成に失敗");
    encoder.finish().expect("finish に失敗");

    // finish 済みの確認は Surface の検証より前に行われるため、Surface の中身は空でもよい。
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    let error = encoder
        .encode(surface, &EncodeOptions::default(), ())
        .expect_err("finish 後の encode が成功した");
    assert_eq!(
        error.status(),
        None,
        "AMF のエラーではなく Encoder::encode のエラーであるべき: {error}"
    );
    assert!(
        error.to_string().contains("already finished"),
        "finish 後の encode のエラーメッセージが想定と異なる: {error}"
    );
}

/// finish 後に reconfigure を呼ぶと、AMF を呼ばずにエラーになることを確認する
#[test]
fn test_reconfigure_after_finish_is_rejected() {
    let mut encoder =
        create_encoder(h264_cbr_test_encoder_config()).expect("エンコーダーの作成に失敗");
    encoder.finish().expect("finish に失敗");

    // 変更する項目を 1 つでも指定し、finish 済みの検証がプロパティの設定より
    // 先に行われることを確認する。
    let params = ReconfigureParams {
        target_kbps: Some(2000),
        ..ReconfigureParams::default()
    };
    let error = encoder
        .reconfigure(params)
        .expect_err("finish 後の reconfigure が成功した");
    assert_eq!(
        error.status(),
        None,
        "AMF のエラーではなく Encoder::reconfigure のエラーであるべき: {error}"
    );
    assert!(
        error.to_string().contains("already finished"),
        "finish 後の reconfigure のエラーメッセージが想定と異なる: {error}"
    );
}

/// finish を 2 回呼ぶと 2 回目がエラーになることを確認する
#[test]
fn test_finish_twice_is_rejected() {
    let mut encoder =
        create_encoder(h264_cbr_test_encoder_config()).expect("エンコーダーの作成に失敗");

    encoder.finish().expect("1 回目の finish に失敗");
    let error = encoder.finish().expect_err("2 回目の finish が成功した");
    assert_eq!(
        error.status(),
        None,
        "AMF のエラーではなく Encoder::finish のエラーであるべき: {error}"
    );
    assert!(
        error.to_string().contains("already finished"),
        "2 回目の finish のエラーメッセージが想定と異なる: {error}"
    );
}

/// finish を呼ばずに drop すると、保留中の入力が通知されないことを確認する
#[test]
fn test_drop_encoder_without_finish_does_not_notify_pending_input() {
    let (first_seen_tx, first_seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let notified = Arc::new(Mutex::new(Vec::new()));

    let mut encoder = Encoder::new(
        create_context(),
        h264_cbr_test_encoder_config(),
        BlockingEncodeHandler {
            first_seen_tx,
            release_rx,
            notified: Arc::clone(&notified),
            blocked: false,
        },
    )
    .expect("エンコーダーの作成に失敗");

    let frame_data = generate_dummy_nv12(320, 240, 0);
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    copy_nv12_to_surface(&surface, &frame_data, 320, 240);
    encoder
        .encode(surface, &EncodeOptions::default(), 0)
        .expect("1 つ目のフレームのエンコードに失敗");

    first_seen_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("最初の通知が届かなかった");

    // ワーカーは最初の通知の処理中でチャネルを読まないため、この入力は保留になる。
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    copy_nv12_to_surface(&surface, &frame_data, 320, 240);
    encoder
        .encode(surface, &EncodeOptions::default(), 1)
        .expect("2 つ目のフレームのエンコードに失敗");

    // drop が cmd_tx を切断してから最初の通知を解放する。解放後のワーカーは
    // 保留中の入力と取得済みの出力を通知せずに終了する。
    let release_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        release_tx.send(()).expect("解放の送信に失敗");
    });

    drop(encoder);
    release_thread.join().expect("解放スレッドの join に失敗");

    let notified = Arc::try_unwrap(notified)
        .expect("通知一覧の Arc の参照が残っている")
        .into_inner()
        .expect("通知一覧のロックに失敗");
    assert_eq!(
        notified,
        vec![0],
        "finish を呼ばずに drop したのに保留中の入力が通知された"
    );
}

/// P010 のフレームデータを Surface に書き込む
///
/// Y プレーンはフレーム番号に応じたグラデーション、UV プレーンは 0x8000 固定にする。
fn write_p010_surface(surface: &Surface, width: usize, height: usize, frame_index: usize) {
    let mut y_plane = surface
        .get_plane(PlaneType::Y)
        .expect("Y プレーンの取得に失敗");
    // get_hpitch() はバイト単位なので、そのまま行の先頭位置の計算に使う
    let y_pitch = y_plane.get_hpitch() as usize;
    let y_data = unsafe { y_plane.as_mut_slice() }.expect("Y プレーンのスライス取得に失敗");
    for row in 0..height {
        let row_start = row * y_pitch;
        let row_bytes = &mut y_data[row_start..row_start + width * 2];
        for (col, sample) in row_bytes.chunks_exact_mut(2).enumerate() {
            // 10bit サンプルを 16bit ワードの上位ビットに格納する
            let value = (((col + row + frame_index * 7) % 256) as u16) << 6;
            sample.copy_from_slice(&value.to_ne_bytes());
        }
    }

    let mut uv_plane = surface
        .get_plane(PlaneType::Uv)
        .expect("UV プレーンの取得に失敗");
    let uv_pitch = uv_plane.get_hpitch() as usize;
    let uv_data = unsafe { uv_plane.as_mut_slice() }.expect("UV プレーンのスライス取得に失敗");
    for row in 0..height / 2 {
        let row_start = row * uv_pitch;
        let row_bytes = &mut uv_data[row_start..row_start + width * 2];
        for sample in row_bytes.chunks_exact_mut(2) {
            // 10bit の無彩色 (512) を 16bit ワードの上位ビットに格納する
            sample.copy_from_slice(&(512u16 << 6).to_ne_bytes());
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

/// HEVC には B フレーム用の QP プロパティがないため、qpb を指定すると Encoder::new がエラーになることを確認する
#[test]
fn test_encoder_rejects_hevc_qpb() {
    let mut config = test_encoder_config(hevc_main(), FrameFormat::Nv12);
    config.qpb = Some(30);
    let err = create_encoder(config)
        .err()
        .expect("HEVC に qpb を指定したのにエンコーダーを作成できた");
    assert_eq!(
        err.status(),
        None,
        "AMF のエラーではなく Encoder::new のエラーであるべき: {err}"
    );
    assert!(
        err.to_string().contains("qpb is not supported for HEVC"),
        "想定したエラーメッセージではない: {err}"
    );
}

/// HEVC の AMF エンコーダーは B フレームを扱わないため、b_pic_pattern を指定すると
/// Encoder::new がエラーになることを確認する
#[test]
fn test_encoder_rejects_hevc_b_pic_pattern() {
    let mut config = test_encoder_config(hevc_main(), FrameFormat::Nv12);
    config.b_pic_pattern = Some(2);
    let err = create_encoder(config)
        .err()
        .expect("HEVC に b_pic_pattern を指定したのにエンコーダーを作成できた");
    assert_eq!(
        err.status(),
        None,
        "AMF のエラーではなく Encoder::new のエラーであるべき: {err}"
    );
    assert!(
        err.to_string()
            .contains("b_pic_pattern is not supported for HEVC"),
        "想定したエラーメッセージではない: {err}"
    );
}

/// B フレームに対応していない GPU では、B フレームを要求すると Encoder::new が
/// エラーになることを確認する
///
/// AMF は非対応の GPU でもプロパティ設定を受理して B フレームを生成しないため、
/// 要求が黙って無視されないことを確認する。
#[test]
fn test_encoder_rejects_b_frames_not_supported_by_gpu() {
    let Some(av1_encoding) = supported_codecs()
        .expect("コーデック対応状況の取得に失敗")
        .into_iter()
        .find(|info| info.codec == VideoCodecType::Av1)
        .and_then(|info| info.encoding)
    else {
        eprintln!("AV1 エンコーダーに対応していないためテストをスキップする");
        return;
    };
    match av1_encoding.supports_frame_reordering {
        Some(false) => {}
        Some(true) => {
            eprintln!("AV1 の B フレームに対応しているためテストをスキップする");
            return;
        }
        None => {
            eprintln!("AV1 の B フレーム対応情報を取得できないためテストをスキップする");
            return;
        }
    }

    let mut config = test_encoder_config(av1_main(), FrameFormat::Nv12);
    config.b_pic_pattern = Some(2);
    let err = create_encoder(config)
        .err()
        .expect("B フレーム非対応の GPU なのにエンコーダーを作成できた");
    assert_eq!(
        err.status(),
        None,
        "AMF のエラーではなく Encoder::new のエラーであるべき: {err}"
    );
    assert!(
        err.to_string()
            .contains("B frames are not supported by this GPU"),
        "想定したエラーメッセージではない: {err}"
    );
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

/// すべてのレート制御モードでエンコーダーを作成し、数フレームをエンコードできることを確認する
///
/// レート制御メソッドの enum 値は H.264 と HEVC / AV1 で並びが異なるため、
/// H.264 / HEVC / AV1 のすべてのモードについて、実際の AMF がその値を受け付けて
/// 出力を返すことを確認する。設定値そのものの同一性は bindgen が生成する enum の
/// 定義で担保する。
#[test]
fn test_encoder_all_rate_control_modes() {
    let context = create_context();
    let codecs = [
        (
            "H.264",
            CodecConfig::H264(H264EncoderConfig {
                profile: Some(H264Profile::High),
            }),
        ),
        ("HEVC", hevc_main()),
        ("AV1", av1_main()),
    ];
    let modes = [
        RateControlMode::Cqp,
        RateControlMode::Cbr,
        RateControlMode::Vbr,
        RateControlMode::LatencyConstrainedVbr,
        RateControlMode::QualityVbr,
        RateControlMode::HighQualityVbr,
        RateControlMode::HighQualityCbr,
    ];

    for (codec_name, codec) in codecs {
        for mode in modes {
            let mut config =
                EncoderConfig::new(codec.clone(), 320, 240, FrameFormat::Nv12, 30, 1, mode);
            // CQP は QP、それ以外のモードはビットレートでレートを制御する。
            // 各モードを初期化できるよう、QP またはビットレートを指定する。
            match mode {
                RateControlMode::Cqp => {
                    config.qpi = Some(26);
                    config.qpp = Some(28);
                }
                _ => {
                    config.target_kbps = Some(1000);
                    config.max_kbps = Some(1000);
                }
            }

            let (tx, rx) = mpsc::channel();
            let mut encoder = Encoder::new(
                context.clone(),
                config,
                FnEncodeHandler::<()>::new(move |result| {
                    tx.send(result).expect("エンコード結果の送信に失敗");
                }),
            )
            .unwrap_or_else(|error| {
                panic!("{codec_name} {mode:?}: エンコーダーの作成に失敗: {error}")
            });

            let num_frames = 3;
            for frame_index in 0..num_frames {
                let surface = encoder.alloc_surface().unwrap_or_else(|error| {
                    panic!("{codec_name} {mode:?}: Surface の確保に失敗: {error}")
                });
                let data = generate_dummy_nv12(320, 240, frame_index);
                copy_nv12_to_surface(&surface, &data, 320, 240);
                encoder
                    .encode(surface, &EncodeOptions::default(), ())
                    .unwrap_or_else(|error| {
                        panic!(
                            "{codec_name} {mode:?}: フレーム {frame_index} のエンコードに失敗: {error}"
                        )
                    });
            }
            encoder
                .finish()
                .unwrap_or_else(|error| panic!("{codec_name} {mode:?}: finish に失敗: {error}"));
            drop(encoder);

            let frames: Vec<_> = rx.try_iter().collect();
            assert_eq!(
                frames.len(),
                num_frames,
                "{codec_name} {mode:?}: エンコードされたフレーム数が一致しない"
            );
            for (frame_index, result) in frames.iter().enumerate() {
                let frame = result.as_ref().unwrap_or_else(|error| {
                    panic!(
                        "{codec_name} {mode:?}: フレーム {frame_index} のエンコードに失敗: {error}"
                    )
                });
                assert!(
                    frame.buffer().get_size() > 0,
                    "{codec_name} {mode:?}: フレーム {frame_index} のビットストリームが空"
                );
            }
        }
    }
}
