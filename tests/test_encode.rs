use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use shiguredo_amf::amf::Surface;
use shiguredo_amf::ffi::AMF_RESULT;
use shiguredo_amf::{
    AmfError, Av1EncoderConfig, Av1Profile, CodecConfig, EncodeHandler, EncodeOptions,
    EncodedFrame, Encoder, EncoderConfig, Error, FnEncodeHandler, FrameFormat, H264EncoderConfig,
    H264Profile, HevcEncoderConfig, HevcProfile, PlaneType, RateControlMode, ReconfigureParams,
    VideoCodecType, supported_codecs,
};

pub mod helpers;

use helpers::{
    copy_nv12_to_surface, create_context, generate_dummy_nv12, h264_cbr_test_encoder_config,
};

/// AMF のエラーから分類型を取り出す
///
/// AMF の呼び出しに起因しないエラーが返された場合は、`context` を付けてテストを失敗させる。
fn amf_error<'a>(error: &'a Error, context: &str) -> &'a AmfError {
    match error {
        Error::Amf(error) => error,
        other => panic!("{context}: AMF のエラーではない: {other:?}"),
    }
}

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

/// 最初の通知でブロックするハンドラー
///
/// Drop 中にキュー済みの入力が処理されても、通知された `user_data` を記録できる。
struct BlockingDropEncodeHandler {
    /// 最初の通知を送るチャネル
    first_seen_tx: mpsc::Sender<()>,
    /// 最初の通知の処理を再開するチャネル
    release_rx: mpsc::Receiver<()>,
    /// 通知された `user_data` の一覧
    notified: Arc<Mutex<Vec<usize>>>,
    /// 最初の通知を処理中か
    blocked: bool,
}

impl EncodeHandler for BlockingDropEncodeHandler {
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

/// 最初の通知で panic するハンドラー
struct PanicEncodeHandler;

impl EncodeHandler for PanicEncodeHandler {
    type UserData = ();
    type Error = Error;

    fn on_encoded(&mut self, _result: Result<EncodedFrame<()>, Error>) {
        panic!("テスト用のエンコードハンドラー panic");
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
        .encode(surface, EncodeOptions::default(), ())
        .expect_err("finish 後の encode が成功した");
    assert!(
        matches!(&error, Error::InvalidState(_)),
        "Encoder::encode の状態エラーであるべき: {error}"
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
    assert!(
        matches!(&error, Error::InvalidState(_)),
        "Encoder::reconfigure の状態エラーであるべき: {error}"
    );
}

/// finish を 2 回呼ぶと 2 回目がエラーになることを確認する
#[test]
fn test_finish_twice_is_rejected() {
    let mut encoder =
        create_encoder(h264_cbr_test_encoder_config()).expect("エンコーダーの作成に失敗");

    encoder.finish().expect("1 回目の finish に失敗");
    let error = encoder.finish().expect_err("2 回目の finish が成功した");
    assert!(
        matches!(&error, Error::InvalidState(_)),
        "Encoder::finish の状態エラーであるべき: {error}"
    );
}

/// finish を呼ばずに drop しても、キュー済みの入力を処理して停止することを確認する
///
/// drop の処理中に通知されるフレームがあっても、通知される `user_data` は送信済みの
/// フレームの範囲に収まり、重複しないことを確認する。
#[test]
fn test_drop_encoder_without_finish_does_not_hang() {
    let (first_seen_tx, first_seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let notified = Arc::new(Mutex::new(Vec::new()));

    let mut encoder = Encoder::new(
        create_context(),
        h264_cbr_test_encoder_config(),
        BlockingDropEncodeHandler {
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
        .encode(surface, EncodeOptions::default(), 0)
        .expect("1 つ目のフレームのエンコードに失敗");

    first_seen_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("最初の通知が届かなかった");

    // ワーカーは最初の通知の処理中なので、この入力はキューに残る。
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    copy_nv12_to_surface(&surface, &frame_data, 320, 240);
    encoder
        .encode(surface, EncodeOptions::default(), 1)
        .expect("2 つ目のフレームのエンコードに失敗");

    // drop はワーカーの停止まで待つため、ブロック中の通知を別スレッドから解放する。
    let release_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        release_tx.send(()).expect("解放の送信に失敗");
    });

    let started = Instant::now();
    drop(encoder);
    let elapsed = started.elapsed();
    release_thread.join().expect("解放スレッドの join に失敗");

    assert!(
        elapsed < Duration::from_secs(5),
        "Encoder の drop に時間がかかりすぎた: {elapsed:?}"
    );

    let notified = Arc::try_unwrap(notified)
        .expect("通知一覧の Arc の参照が残っている")
        .into_inner()
        .expect("通知一覧のロックに失敗");
    assert!(
        notified.contains(&0),
        "1 つ目のフレームが通知されていない: {notified:?}"
    );
    assert!(
        notified.iter().all(|index| *index <= 1),
        "送信していないフレームが通知された: {notified:?}"
    );
    let mut unique = notified.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        notified.len(),
        "同じフレームが複数回通知された: {notified:?}"
    );
}

/// finish 中にハンドラーが panic した場合、finish が Err を返すことを確認する
#[test]
fn test_finish_returns_error_when_handler_panics() {
    let mut encoder = Encoder::new(
        create_context(),
        h264_cbr_test_encoder_config(),
        PanicEncodeHandler,
    )
    .expect("エンコーダーの作成に失敗");

    let frame_data = generate_dummy_nv12(320, 240, 0);
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    copy_nv12_to_surface(&surface, &frame_data, 320, 240);
    // ハンドラーの panic は encode 中にも起こり得るため、encode の成否は問わない。
    let _ = encoder.encode(surface, EncodeOptions::default(), ());

    let error = encoder
        .finish()
        .expect_err("ハンドラーが panic したのに finish が成功した");
    assert!(
        matches!(error, Error::Internal(_)),
        "ハンドラー panic 時の finish エラーが Internal でない: {error}"
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
            assert!(
                matches!(&err, Error::InvalidArgument(_)),
                "{name} {frame_format:?}: Encoder::new の引数エラーであるべき: {err}"
            );
            let message = err.to_string();
            assert!(
                message.contains("HEVC 10bit input requires HevcProfile::Main10"),
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
    assert!(
        matches!(&err, Error::InvalidArgument(_)),
        "Encoder::new の引数エラーであるべき: {err}"
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
    assert!(
        matches!(&err, Error::InvalidArgument(_)),
        "Encoder::new の引数エラーであるべき: {err}"
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
    assert!(
        matches!(&err, Error::InvalidArgument(_)),
        "Encoder::new の引数エラーであるべき: {err}"
    );
    assert!(
        err.to_string().contains("do not support B frames"),
        "想定したエラーメッセージではない: {err}"
    );
}

/// Encoder::new が不正な解像度とフレームレートを InvalidArgument で拒否することを確認する
#[test]
fn test_encoder_rejects_invalid_config() {
    let cases = [
        ("zero width", 0, 240, 30, 1, "invalid resolution"),
        ("odd height", 320, 241, 30, 1, "invalid resolution"),
        (
            "width exceeds i32::MAX",
            i32::MAX as u32 + 1,
            240,
            30,
            1,
            "invalid resolution",
        ),
        (
            "zero framerate denominator",
            320,
            240,
            30,
            0,
            "invalid framerate",
        ),
    ];
    for (name, width, height, framerate_num, framerate_den, expected_message) in cases {
        let mut config = h264_cbr_test_encoder_config();
        config.width = width;
        config.height = height;
        config.framerate_num = framerate_num;
        config.framerate_den = framerate_den;
        let error = Encoder::new(create_context(), config, FnEncodeHandler::<()>::new(|_| {}))
            .err()
            .expect("不正な設定のエンコーダーを作成できた");
        assert!(
            matches!(&error, Error::InvalidArgument(_)),
            "{name}: InvalidArgument ではない: {error}"
        );
        assert!(
            error.to_string().contains(expected_message),
            "{name}: エラーメッセージが想定と異なる: {error}"
        );
    }
}

/// Encoder::reconfigure がフレームレートの片方だけの指定を InvalidArgument で拒否することを確認する
#[test]
fn test_reconfigure_rejects_incomplete_framerate() {
    let mut encoder =
        create_encoder(h264_cbr_test_encoder_config()).expect("エンコーダーの作成に失敗");
    let error = encoder
        .reconfigure(ReconfigureParams {
            framerate_num: Some(30),
            ..ReconfigureParams::default()
        })
        .expect_err("フレームレートの分子だけを指定した reconfigure が成功した");
    assert!(
        matches!(&error, Error::InvalidArgument(_)),
        "InvalidArgument ではない: {error}"
    );
    assert!(
        error.to_string().contains("must be set together"),
        "エラーメッセージが想定と異なる: {error}"
    );
    encoder.finish().expect("finish に失敗");
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
                let error = amf_error(&e, &format!("{name}: P010"));
                assert_eq!(
                    error.status(),
                    AMF_RESULT::AMF_INVALID_FORMAT,
                    "{name}: P010 のエンコーダーが想定外のステータスで失敗した: {e}"
                );
                assert_eq!(
                    error.operation(),
                    "AMFComponent::Init",
                    "{name}: P010 のエンコーダーが Init 以外の AMF 呼び出しで失敗した: {e}"
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
                .encode(surface, EncodeOptions::default(), ())
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
                    let error = amf_error(&e, &format!("{name} {frame_format:?}"));
                    assert_eq!(
                        error.status(),
                        AMF_RESULT::AMF_INVALID_FORMAT,
                        "{name} {frame_format:?}: 想定外のステータスで失敗した: {e}"
                    );
                    assert_eq!(
                        error.operation(),
                        "AMFComponent::Init",
                        "{name} {frame_format:?}: Init 以外の AMF 呼び出しで失敗した: {e}"
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
                    .encode(surface, EncodeOptions::default(), ())
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

/// 出力の回収を待たずに大量のフレームを投入し、すべてのフレームが出力されることを確認する
///
/// `SubmitInput` のリトライ中にワーカーが出力を回収できないと、リトライ上限に
/// 達して `encode` がエラーになる。フレームを待たずに連続投入しても各 `encode` が
/// 成功し、投入したすべてのフレームの出力が得られることを確認する。
#[test]
fn test_encoder_continuous_submit_outputs_all_frames() {
    /// 連続投入するフレーム数
    const NUM_FRAMES: usize = 200;

    let config = h264_cbr_test_encoder_config();
    let width = config.width as usize;
    let height = config.height as usize;

    let (tx, rx) = mpsc::channel();
    let mut encoder = Encoder::new(
        create_context(),
        config,
        FnEncodeHandler::<usize>::new(move |result| {
            tx.send(result).expect("エンコード結果の送信に失敗");
        }),
    )
    .expect("エンコーダーの作成に失敗");

    // 入力側の処理を軽くして投入の間隔を短くする。Surface の確保をループの
    // 外に出し、フレームの内容は毎回同じものを使う。
    let frame = generate_dummy_nv12(width, height, 0);
    let mut surfaces = Vec::new();
    for frame_index in 0..NUM_FRAMES {
        let surface = encoder.alloc_surface().unwrap_or_else(|error| {
            panic!("フレーム {frame_index} の Surface の確保に失敗: {error}")
        });
        surfaces.push(surface);
    }
    for (frame_index, surface) in surfaces.into_iter().enumerate() {
        copy_nv12_to_surface(&surface, &frame, width, height);
        encoder
            .encode(surface, EncodeOptions::default(), frame_index)
            .unwrap_or_else(|error| panic!("フレーム {frame_index} のエンコードに失敗: {error}"));
    }
    encoder.finish().expect("finish に失敗");
    drop(encoder);

    let frames: Vec<_> = rx.try_iter().collect();
    assert_eq!(
        frames.len(),
        NUM_FRAMES,
        "エンコードされたフレーム数が一致しない"
    );

    let mut user_data = Vec::new();
    for (frame_index, result) in frames.into_iter().enumerate() {
        let frame = result.unwrap_or_else(|error| {
            panic!("フレーム {frame_index} のエンコード結果がエラーになった: {error}")
        });
        assert!(
            frame.buffer().get_size() > 0,
            "フレーム {frame_index} のビットストリームが空"
        );
        user_data.push(*frame.user_data());
    }
    user_data.sort_unstable();
    assert_eq!(
        user_data,
        (0..NUM_FRAMES).collect::<Vec<_>>(),
        "出力に重複または欠落した user_data がある"
    );
}

/// 統計値が投入数、出力数、出力ビット数、再設定回数と一致することを確認する
#[test]
fn test_encoder_stats_counts() {
    /// 投入するフレーム数
    const NUM_FRAMES: usize = 10;

    let config = h264_cbr_test_encoder_config();
    let width = config.width as usize;
    let height = config.height as usize;

    let (tx, rx) = mpsc::channel();
    let mut encoder = Encoder::new(
        create_context(),
        config,
        FnEncodeHandler::<usize>::new(move |result| {
            tx.send(result).expect("エンコード結果の送信に失敗");
        }),
    )
    .expect("エンコーダーの作成に失敗");

    // 作成直後はどの統計値も 0 である。
    assert_eq!(
        encoder.stats().total_encode_count.get(),
        0,
        "作成直後の total_encode_count が 0 ではない"
    );
    assert_eq!(
        encoder.stats().total_output_frame_count.get(),
        0,
        "作成直後の total_output_frame_count が 0 ではない"
    );
    assert_eq!(
        encoder.stats().total_error_count.get(),
        0,
        "作成直後の total_error_count が 0 ではない"
    );
    assert_eq!(
        encoder.stats().total_output_bit_count.get(),
        0,
        "作成直後の total_output_bit_count が 0 ではない"
    );
    assert_eq!(
        encoder.stats().total_reconfigure_count.get(),
        0,
        "作成直後の total_reconfigure_count が 0 ではない"
    );
    assert_eq!(
        encoder.stats().in_flight_frames.get(),
        0,
        "作成直後の in_flight_frames が 0 ではない"
    );

    // 動的プロパティの再設定に成功した回数が計上されることを確認する。
    encoder
        .reconfigure(ReconfigureParams {
            target_kbps: Some(800),
            ..ReconfigureParams::default()
        })
        .expect("reconfigure に失敗");
    assert_eq!(
        encoder.stats().total_reconfigure_count.get(),
        1,
        "reconfigure 成功後の total_reconfigure_count が 1 ではない"
    );

    let frame = generate_dummy_nv12(width, height, 0);
    for frame_index in 0..NUM_FRAMES {
        let surface = encoder.alloc_surface().unwrap_or_else(|error| {
            panic!("フレーム {frame_index} の Surface の確保に失敗: {error}")
        });
        copy_nv12_to_surface(&surface, &frame, width, height);
        encoder
            .encode(surface, EncodeOptions::default(), frame_index)
            .unwrap_or_else(|error| panic!("フレーム {frame_index} のエンコードに失敗: {error}"));
    }

    // encode が成功した時点で、投入したフレームが計上される。
    assert_eq!(
        encoder.stats().total_encode_count.get(),
        NUM_FRAMES as u64,
        "投入後の total_encode_count が投入数と一致しない"
    );

    encoder.finish().expect("finish に失敗");

    // finish の完了時点で、受理したすべてのフレームの結果が通知されている。
    assert_eq!(
        encoder.stats().in_flight_frames.get(),
        0,
        "finish 後の in_flight_frames が 0 ではない"
    );
    assert_eq!(
        encoder.stats().total_encode_count.get(),
        NUM_FRAMES as u64,
        "finish 後の total_encode_count が投入数と一致しない"
    );
    assert_eq!(
        encoder.stats().total_output_frame_count.get(),
        NUM_FRAMES as u64,
        "finish 後の total_output_frame_count が投入数と一致しない"
    );
    assert_eq!(
        encoder.stats().total_error_count.get(),
        0,
        "finish 後の total_error_count が 0 ではない"
    );
    assert_eq!(
        encoder.stats().total_reconfigure_count.get(),
        1,
        "finish 後の total_reconfigure_count が 1 ではない"
    );

    let frames: Vec<_> = rx.try_iter().collect();
    assert_eq!(
        frames.len(),
        NUM_FRAMES,
        "エンコードされたフレーム数が一致しない"
    );
    let mut total_bytes = 0;
    for (frame_index, result) in frames.iter().enumerate() {
        let frame = result.as_ref().unwrap_or_else(|error| {
            panic!("フレーム {frame_index} のエンコード結果がエラーになった: {error}")
        });
        total_bytes += frame.buffer().get_size() as u64;
    }

    // total_output_bit_count は出力バッファのバイト数をビット数に換算した値になる。
    assert_eq!(
        encoder.stats().total_output_bit_count.get(),
        total_bytes * 8,
        "total_output_bit_count が出力バッファのビット数と一致しない"
    );
}
