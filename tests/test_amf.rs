use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use shiguredo_amf::amf::{Buffer, Component, Context, Plane, PropertyStorage, Surface};
use shiguredo_amf::ffi::{AMF_PLANE_TYPE, amf_pts};
use shiguredo_amf::{
    AmfLibrary, CodecConfig, Decoder, DecoderCodec, DecoderConfig, EncodeOptions, Encoder,
    EncoderConfig, FnDecodeHandler, FnEncodeHandler, FrameFormat, H264EncoderConfig, H264Profile,
    RateControlMode, frame_type,
};

/// 型が `Clone` であることをコンパイル時に表明する
fn assert_clone<T: Clone>() {}

/// 型が `Send + Sync` であることをコンパイル時に表明する
fn assert_send_sync<T: Send + Sync>() {}

/// NV12 のダミーフレームを Surface に書き込む
///
/// Y プレーンは座標に応じたグラデーション、UV プレーンは 128 固定にする。
fn fill_nv12_surface(surface: &Surface, width: usize, height: usize) {
    let y_plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");
    let y_native = y_plane.get_native() as *mut u8;
    let y_hpitch = y_plane.get_hpitch() as usize;
    for row in 0..height {
        for col in 0..width {
            let value = ((row + col) % 256) as u8;
            unsafe {
                *y_native.add(row * y_hpitch + col) = value;
            }
        }
    }

    let uv_plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_UV)
        .expect("UV プレーンの取得に失敗");
    let uv_native = uv_plane.get_native() as *mut u8;
    let uv_hpitch = uv_plane.get_hpitch() as usize;
    for row in 0..height / 2 {
        for col in 0..width {
            unsafe {
                *uv_native.add(row * uv_hpitch + col) = 128;
            }
        }
    }
}

/// `amf` モジュールの全ハンドルが `Clone + Send + Sync` であることを確認する
#[test]
fn test_amf_handles_are_clone_send_sync() {
    // AMF ランタイムを実際に利用できる環境で実行する前提のテストであるため、
    // ランタイムのバージョン照会が成功することを確認する。
    AmfLibrary::instance()
        .query_version()
        .expect("AMF ランタイムのバージョン取得に失敗");

    assert_clone::<Context>();
    assert_clone::<Component>();
    assert_clone::<Surface>();
    assert_clone::<Buffer>();
    assert_clone::<Plane>();
    assert_clone::<PropertyStorage>();

    assert_send_sync::<Context>();
    assert_send_sync::<Component>();
    assert_send_sync::<Surface>();
    assert_send_sync::<Buffer>();
    assert_send_sync::<Plane>();
    assert_send_sync::<PropertyStorage>();
}

/// 同じ `Surface` を指す Clone を複数スレッドで並行利用しても安全であることを確認する
///
/// `Plane` と `PropertyStorage` は `Surface` の共有ロックを共有するため、
/// これらを並行して操作してもサーフェスの状態が壊れないことを確認する。
#[test]
fn test_surface_clone_concurrent_access() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        WIDTH as u32,
        HEIGHT as u32,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cqp,
    );
    config.qpi = Some(26);
    config.qpp = Some(28);
    config.qpb = Some(30);
    config.gop_pic_size = Some(30);

    let mut encoder =
        Encoder::new(config, FnEncodeHandler::<()>::new(|_| {})).expect("エンコーダーの作成に失敗");
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

                    // PropertyStorage も Surface と同じ共有ロックを共有する。
                    // 存在しないプロパティへの設定は失敗するが、ここでは共有ロックの
                    // 下で FFI 呼び出しが直列化されることだけを確認する。
                    let prop = surface
                        .property_storage()
                        .expect("PropertyStorage の取得に失敗");
                    let _ = prop.set_property_int64("AmfRustConcurrentTestProperty", i as i64);
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

    // 並行アクセスした Surface をそのままエンコードできることを確認する
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

/// 同じ `Buffer` を指す Clone を複数スレッドで並行利用しても安全であることを確認する
#[test]
fn test_buffer_clone_concurrent_access() {
    const BUFFER_SIZE: usize = 4096;

    let config = DecoderConfig {
        codec: DecoderCodec::H264,
    };
    let decoder =
        Decoder::new(config, FnDecodeHandler::<()>::new(|_| {})).expect("デコーダーの作成に失敗");
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

/// ハンドラー呼び出し中にフレームの共有ロックが保持されていないことを確認する
///
/// エンコードハンドラーでは受け取った `Buffer` の、デコードハンドラーでは
/// 受け取った `Surface` のメソッドを呼ぶ。ワーカースレッドが同じロックを
/// 保持したままハンドラーを呼ぶと、同じスレッドでロックを取得しようとして
/// デッドロックする。
#[test]
fn test_handlers_can_access_frames() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        WIDTH as u32,
        HEIGHT as u32,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cqp,
    );
    config.qpi = Some(26);
    config.qpp = Some(28);
    config.qpb = Some(30);
    config.gop_pic_size = Some(30);

    // エンコードハンドラーは受け取った Buffer のメソッドを呼ぶ
    let encoded = Arc::new(Mutex::new(Vec::new()));
    let e = Arc::clone(&encoded);
    let mut encoder = Encoder::new(
        config,
        FnEncodeHandler::<()>::new(move |result| {
            let frame = result.expect("エンコード結果の受信に失敗");
            // Buffer の共有ロックが保持されていればここでデッドロックする
            assert!(frame.buffer().get_size() > 0, "エンコード済みバッファが空");
            assert!(
                !frame.buffer().get_native().is_null(),
                "エンコード済みバッファのポインタが null"
            );
            e.lock().unwrap().push(frame.into_parts().0);
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
    drop(encoder);

    let encoded_buffers = Arc::try_unwrap(encoded)
        .expect("Arc の参照が残っている")
        .into_inner()
        .unwrap();
    assert_eq!(
        encoded_buffers.len(),
        2,
        "エンコード結果の数が期待値と一致しない"
    );

    // デコードハンドラーは受け取った Surface のメソッドを呼ぶ
    let decoded_count = Arc::new(AtomicUsize::new(0));
    let observations = Arc::new(Mutex::new(Vec::new()));
    let d = Arc::clone(&decoded_count);
    let o = Arc::clone(&observations);
    let mut decoder = Decoder::new(
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<()>::new(move |result| {
            let observation = match result {
                Ok(frame) => {
                    // Surface の共有ロックが保持されていればここでデッドロックする
                    match frame.surface().get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y) {
                        Ok(plane) => Ok((
                            plane.get_width(),
                            plane.get_height(),
                            !plane.get_native().is_null(),
                        )),
                        Err(err) => Err(format!("Y プレーンの取得に失敗: {err}")),
                    }
                }
                Err(err) => Err(format!("デコード結果の受信に失敗: {err}")),
            };
            o.lock().unwrap().push(observation);
            d.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .expect("デコーダーの作成に失敗");

    for buffer in encoded_buffers {
        decoder.decode(buffer, ()).expect("デコードに失敗");
    }
    decoder.finish().expect("finish に失敗");
    drop(decoder);

    let observations = observations.lock().unwrap();
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
    assert_eq!(
        decoded_count.load(Ordering::SeqCst),
        2,
        "デコード結果の数が期待値と一致しない"
    );
}

/// `into_raw()` が AMF の参照を呼び出し元へ移し、`from_raw()` で再構築できることを確認する
///
/// `into_raw()` が参照を解放してしまうと、再構築したハンドルの利用時に
/// 解放済みのオブジェクトへアクセスすることになる。
#[test]
fn test_surface_into_raw_and_from_raw() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig {
            profile: Some(H264Profile::High),
        }),
        WIDTH as u32,
        HEIGHT as u32,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cqp,
    );
    config.qpi = Some(26);
    config.qpp = Some(28);
    config.qpb = Some(30);

    let encoder =
        Encoder::new(config, FnEncodeHandler::<()>::new(|_| {})).expect("エンコーダーの作成に失敗");
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

    // Drop で参照が解放される
    drop(surface);
    drop(encoder);
}
