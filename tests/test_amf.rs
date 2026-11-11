//! `amf` モジュールのハンドルに関するテスト
//!
//! このファイルのテストは実 GPU と AMF ランタイムを必要とする。
//! テストバイナリを並列実行すると `InitVulkan` の並列初期化で SIGSEGV する
//! 既知の問題があるため、CI では `--test-threads=1` で実行する。

use std::sync::{Arc, Mutex};

use shiguredo_amf::amf::{Buffer, Component, Context, Plane, PropertyStorage, Surface};
use shiguredo_amf::ffi::{AMF_PLANE_TYPE, amf_pts};
use shiguredo_amf::{
    CodecConfig, Decoder, DecoderCodec, DecoderConfig, EncodeOptions, Encoder, EncoderConfig,
    FnDecodeHandler, FnEncodeHandler, FrameFormat, H264EncoderConfig, H264Profile, RateControlMode,
    frame_type,
};

pub mod helpers;

/// 型が `Clone + Send + Sync` であることをコンパイル時に表明する
fn assert_handle<T: Clone + Send + Sync>() {}

/// テスト用の H.264 CQP エンコーダー設定を返す
fn h264_cqp_test_encoder_config() -> EncoderConfig {
    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
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
    config.gop_pic_size = Some(30);
    config
}

/// テスト用の H.264 デコーダー設定を返す
fn h264_test_decoder_config() -> DecoderConfig {
    DecoderConfig {
        codec: DecoderCodec::H264,
    }
}

/// NV12 のダミーフレームを Surface に書き込む
///
/// Y プレーンは座標に応じたグラデーション、UV プレーンは 128 固定にする。
fn fill_nv12_surface(surface: &Surface, width: usize, height: usize) {
    helpers::write_nv12_rows(
        surface,
        width,
        height,
        |row, y_row| {
            for (col, value) in y_row.iter_mut().enumerate() {
                *value = ((row + col) % 256) as u8;
            }
        },
        |_, uv_row| {
            uv_row.fill(128);
        },
    );
}

/// `amf` モジュールの全ハンドルが `Clone + Send + Sync` であることを確認する
#[test]
fn test_amf_handles_are_clone_send_sync() {
    assert_handle::<Context>();
    assert_handle::<Component>();
    assert_handle::<Surface>();
    assert_handle::<Buffer>();
    assert_handle::<Plane>();
    assert_handle::<PropertyStorage>();

    // コンパイル時の表明だけでなく、公開 API から実際にハンドルを作成できることも
    // 確認する。Context と Component は Encoder の内部で作成されるため、ここでは
    // Surface / Plane / PropertyStorage を確認する。
    let encoder = Encoder::new(
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    let plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");
    let property_storage = surface
        .property_storage()
        .expect("PropertyStorage の取得に失敗");

    // 子ハンドルを先に破棄してから Surface と Encoder を破棄する
    drop(property_storage);
    drop(plane);
    drop(surface);
    drop(encoder);
}

/// 同じ `Surface` を指す Clone を複数スレッドで並行利用できることを確認する
///
/// `Plane` と `PropertyStorage` は `Surface` の共有ロックを共有する。
/// このテストは、これらを並行して操作してもクラッシュやデッドロックが起きない
/// ことを確認するスモークテストであり、状態の完全な検証は行わない。
#[test]
fn test_surface_clone_concurrent_access() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    let mut encoder = Encoder::new(
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    fill_nv12_surface(&surface, WIDTH, HEIGHT);

    // Surface の Clone を複数スレッドで並行利用する
    let thread_count = 4;
    let iterations = 100;
    std::thread::scope(|scope| {
        for thread_index in 0..thread_count {
            let surface = surface.clone();
            scope.spawn(move || {
                for i in 0..iterations {
                    let pts = (thread_index * iterations + i) as amf_pts;
                    surface.set_pts(pts);
                    surface.set_duration(pts);

                    let y_plane = surface
                        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
                        .expect("Y プレーンの取得に失敗");
                    assert_eq!(y_plane.get_width(), WIDTH as i32, "幅が一致しない");
                    assert_eq!(y_plane.get_height(), HEIGHT as i32, "高さが一致しない");
                    assert!(
                        y_plane.get_hpitch() >= WIDTH as i32,
                        "水平ピッチが幅より小さい"
                    );
                    assert!(!y_plane.get_native().is_null(), "Y プレーンが null");

                    // AMFPropertyStorage::SetProperty は存在しないプロパティでも
                    // AMF_OK を返す。ここでは共有ロックの下で FFI 呼び出しが
                    // クラッシュせず、デッドロックしないことだけを確認する。
                    let prop = surface
                        .property_storage()
                        .expect("PropertyStorage の取得に失敗");
                    prop.set_property_int64("AmfRustConcurrentTestProperty", i as i64)
                        .expect("プロパティの設定に失敗");
                }
            });
        }
    });

    // 並行アクセス後も Surface が有効であることを確認する
    let y_plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");
    assert_eq!(y_plane.get_width(), WIDTH as i32, "幅が一致しない");
    assert_eq!(y_plane.get_height(), HEIGHT as i32, "高さが一致しない");

    // コンポーネントに投入する前に、Surface 以外のハンドルをすべて破棄する
    drop(y_plane);

    // 並行アクセスが終わった Surface を、他のハンドルを破棄してからエンコードできることを確認する
    encoder
        .encode(
            surface,
            &EncodeOptions {
                frame_type: frame_type::UNKNOWN,
            },
            (),
        )
        .expect("エンコードに失敗");
    encoder.finish().expect("finish に失敗");
}

/// 同じ `Buffer` を指す Clone からの読み取りを並行してもクラッシュやデッドロックが
/// 起きないことを確認するスモークテスト
///
/// `Buffer` には書き込み系の API がないため、ここでは読み取りの並行呼び出しだけを
/// 確認する。
#[test]
fn test_buffer_clone_concurrent_access() {
    const BUFFER_SIZE: usize = 4096;

    let decoder = Decoder::new(
        h264_test_decoder_config(),
        FnDecodeHandler::<()>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");
    let buffer = decoder
        .alloc_buffer(BUFFER_SIZE)
        .expect("Buffer の確保に失敗");

    // 確保直後の値を記録し、並行アクセス中に変化しないことを確認する
    let pts = buffer.get_pts();
    let duration = buffer.get_duration();

    let thread_count = 4;
    let iterations = 100;
    std::thread::scope(|scope| {
        for _ in 0..thread_count {
            let buffer = buffer.clone();
            scope.spawn(move || {
                for _ in 0..iterations {
                    assert_eq!(buffer.get_size(), BUFFER_SIZE, "サイズが一致しない");
                    assert!(!buffer.get_native().is_null(), "バッファが null");
                    assert_eq!(buffer.get_pts(), pts, "PTS が変化した");
                    assert_eq!(buffer.get_duration(), duration, "Duration が変化した");
                }
            });
        }
    });

    // 並行アクセス後も Buffer が有効であることを確認する
    assert_eq!(buffer.get_size(), BUFFER_SIZE, "サイズが一致しない");
}

/// ハンドラーからフレームのメソッドを呼べることを確認する
///
/// ワーカーが共有ロックを保持したままハンドラーを呼ぶ実装では、ハンドラー内の
/// 同じハンドルへのアクセスでデッドロックする。このテストはその実装になって
/// いないことを確認する。
#[test]
fn test_handlers_can_access_frames() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    // エンコードハンドラーは受け取った Buffer のメソッドを呼ぶ
    let encoded = Arc::new(Mutex::new(Vec::new()));
    let encode_observations = Arc::new(Mutex::new(Vec::new()));
    let e = Arc::clone(&encoded);
    let eo = Arc::clone(&encode_observations);
    let mut encoder = Encoder::new(
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(move |result| {
            let observation = match result {
                Ok(frame) => {
                    let size = frame.buffer().get_size();
                    let native_ok = !frame.buffer().get_native().is_null();
                    e.lock()
                        .expect("エンコード結果の記録用 Mutex がポイズンされた")
                        .push(frame.into_parts().0);
                    Ok((size, native_ok))
                }
                Err(err) => Err(format!("エンコード結果の受信に失敗: {err}")),
            };
            eo.lock()
                .expect("エンコード観察の記録用 Mutex がポイズンされた")
                .push(observation);
        }),
    )
    .expect("エンコーダーの作成に失敗");

    let options = EncodeOptions {
        frame_type: frame_type::UNKNOWN,
    };
    for _ in 0..2 {
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        fill_nv12_surface(&surface, WIDTH, HEIGHT);
        encoder
            .encode(surface, &options, ())
            .expect("エンコードに失敗");
    }
    encoder.finish().expect("finish に失敗");

    // ワーカーを join してハンドラーが保持する Arc を落とす
    drop(encoder);

    let encode_observations = Arc::try_unwrap(encode_observations)
        .expect("エンコード観察の Arc の参照が残っている")
        .into_inner()
        .expect("エンコード観察の Mutex がポイズンされた");
    assert_eq!(
        encode_observations.len(),
        2,
        "エンコード結果の数が期待値と一致しない"
    );
    for (i, observation) in encode_observations.iter().enumerate() {
        let (size, native_ok) = observation
            .as_ref()
            .unwrap_or_else(|err| panic!("エンコード結果 {i}: {err}"));
        assert!(*size > 0, "エンコード済みバッファ {i} が空");
        assert!(*native_ok, "エンコード済みバッファ {i} のポインタが null");
    }

    let encoded_buffers = Arc::try_unwrap(encoded)
        .expect("エンコード結果の Arc の参照が残っている")
        .into_inner()
        .expect("エンコード結果の Mutex がポイズンされた");

    // デコードハンドラーは受け取った Surface のメソッドを呼ぶ
    let observations = Arc::new(Mutex::new(Vec::new()));
    let o = Arc::clone(&observations);
    let mut decoder = Decoder::new(
        h264_test_decoder_config(),
        FnDecodeHandler::<()>::new(move |result| {
            let observation = match result {
                Ok(frame) => match frame.surface().get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y) {
                    Ok(plane) => Ok((
                        plane.get_width(),
                        plane.get_height(),
                        !plane.get_native().is_null(),
                    )),
                    Err(err) => Err(format!("Y プレーンの取得に失敗: {err}")),
                },
                Err(err) => Err(format!("デコード結果の受信に失敗: {err}")),
            };
            o.lock()
                .expect("デコード観察の記録用 Mutex がポイズンされた")
                .push(observation);
        }),
    )
    .expect("デコーダーの作成に失敗");

    for buffer in encoded_buffers {
        decoder.decode(buffer, ()).expect("デコードに失敗");
    }
    decoder.finish().expect("finish に失敗");

    // ワーカーを join してハンドラーが保持する Arc を落とす
    drop(decoder);

    let observations = observations
        .lock()
        .expect("デコード観察用 Mutex がポイズンされた");
    assert_eq!(
        observations.len(),
        2,
        "デコード結果の数が期待値と一致しない"
    );
    for (i, observation) in observations.iter().enumerate() {
        let (width, height, native_ok) = observation
            .as_ref()
            .unwrap_or_else(|err| panic!("デコード済みフレーム {i}: {err}"));
        assert_eq!(
            *width, WIDTH as i32,
            "デコード済みフレーム {i} の幅が一致しない"
        );
        assert_eq!(
            *height, HEIGHT as i32,
            "デコード済みフレーム {i} の高さが一致しない"
        );
        assert!(*native_ok, "デコード済みフレーム {i} の Y プレーンが null");
    }
}

/// `into_raw()` が AMF の参照を呼び出し元へ移し、`from_raw()` で再構築できることを確認する
///
/// `Context` と `Component` は公開 API から安全に構築できないため、ここでは
/// `Surface` / `Buffer` / `Plane` / `PropertyStorage` を対象にする。
#[test]
fn test_surface_into_raw_and_from_raw() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    let encoder = Encoder::new(
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");

    // 参照の所有権を呼び出し元へ移す
    let ptr = surface.into_raw();
    assert!(!ptr.is_null(), "into_raw() が null を返した");

    // 生ポインタからハンドルを再構築できる
    let surface = unsafe { Surface::from_raw(ptr) }.expect("Surface の再構築に失敗");
    let y_plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");
    assert_eq!(y_plane.get_width(), WIDTH as i32, "幅が一致しない");
    assert_eq!(y_plane.get_height(), HEIGHT as i32, "高さが一致しない");

    // 子ハンドルと Surface を Encoder より先に破棄する
    drop(y_plane);
    drop(surface);
    drop(encoder);
}

/// `Buffer` の `into_raw()` と `from_raw()` を確認する
#[test]
fn test_buffer_into_raw_and_from_raw() {
    const BUFFER_SIZE: usize = 4096;

    let decoder = Decoder::new(
        h264_test_decoder_config(),
        FnDecodeHandler::<()>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");
    let buffer = decoder
        .alloc_buffer(BUFFER_SIZE)
        .expect("Buffer の確保に失敗");

    let ptr = buffer.into_raw();
    assert!(!ptr.is_null(), "into_raw() が null を返した");

    let buffer = unsafe { Buffer::from_raw(ptr) }.expect("Buffer の再構築に失敗");
    assert_eq!(buffer.get_size(), BUFFER_SIZE, "サイズが一致しない");
    assert!(!buffer.get_native().is_null(), "バッファが null");

    // Buffer を Decoder より先に破棄する
    drop(buffer);
    drop(decoder);
}

/// `Plane` の `into_raw()` と `from_raw()` を確認する
#[test]
fn test_plane_into_raw_and_from_raw() {
    let encoder = Encoder::new(
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    let plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");

    let ptr = plane.into_raw();
    assert!(!ptr.is_null(), "into_raw() が null を返した");

    let plane = unsafe { Plane::from_raw(ptr) }.expect("Plane の再構築に失敗");
    assert_eq!(plane.get_width(), 320, "幅が一致しない");
    assert_eq!(plane.get_height(), 240, "高さが一致しない");

    // Plane を Surface より先に、Surface を Encoder より先に破棄する
    drop(plane);
    drop(surface);
    drop(encoder);
}

/// `PropertyStorage` の `into_raw()` と `from_raw()` を確認する
#[test]
fn test_property_storage_into_raw_and_from_raw() {
    let encoder = Encoder::new(
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    let property_storage = surface
        .property_storage()
        .expect("PropertyStorage の取得に失敗");

    let ptr = property_storage.into_raw();
    assert!(!ptr.is_null(), "into_raw() が null を返した");

    let property_storage =
        unsafe { PropertyStorage::from_raw(ptr) }.expect("PropertyStorage の再構築に失敗");
    property_storage
        .set_property_int64("AmfRustIntoRawTestProperty", 1)
        .expect("プロパティの設定に失敗");

    // PropertyStorage を Surface より先に、Surface を Encoder より先に破棄する
    drop(property_storage);
    drop(surface);
    drop(encoder);
}
