//! `amf` モジュールのハンドルに関するテスト
//!
//! このファイルのテストは実 GPU と AMF ランタイムを必要とする。

use std::sync::{Arc, Barrier, Mutex};

use shiguredo_amf::amf::{Buffer, Component, Context, Plane, PropertyStorage, Surface};
use shiguredo_amf::ffi::{
    AMF_MEMORY_TYPE, AMF_PLANE_TYPE, AMF_RESULT, AMF_SURFACE_FORMAT, amf_long, amf_pts,
};
use shiguredo_amf::{
    CodecConfig, Decoder, DecoderCodec, DecoderConfig, EncodeOptions, Encoder, EncoderConfig,
    FnDecodeHandler, FnEncodeHandler, FrameFormat, H264EncoderConfig, H264Profile, RateControlMode,
    frame_type,
};

pub mod helpers;

/// 型が `Clone + Send + Sync` であることをコンパイル時に表明する
fn assert_handle<T: Clone + Send + Sync>() {}

/// 型が `Send + Sync` であることをコンパイル時に表明する
fn assert_send_sync<T: Send + Sync>() {}

/// Context が保持する AMFContext の参照カウントを取得する
fn context_ref_count(context: &Context) -> amf_long {
    unsafe {
        let ptr = context.as_ptr();
        let vtbl = &*(*ptr).pVtbl;
        let acquire = vtbl.Acquire.expect("AMFContext::Acquire がない");
        let release = vtbl.Release.expect("AMFContext::Release がない");
        let count = acquire(ptr);
        release(ptr);
        count - 1
    }
}

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

/// `amf` モジュールのハンドルが `Send + Sync` / `Clone + Send + Sync` であることを確認する
#[test]
fn test_amf_handles_are_send_sync() {
    // Context は Clone で AMF の参照カウントを増やせる。
    assert_send_sync::<Context>();
    assert_handle::<Context>();
    assert_handle::<Component>();
    assert_handle::<Surface>();
    assert_handle::<Buffer>();
    assert_handle::<Plane>();
    assert_handle::<PropertyStorage>();

    // コンパイル時の表明だけでなく、公開 API から実際にハンドルを作成できることも
    // 確認する。Component は Encoder の内部で作成されるため、ここでは Context と
    // Surface / Plane / PropertyStorage を確認する。
    let context = helpers::create_context();
    let encoder = Encoder::new(
        context.clone(),
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

    let context = helpers::create_context();
    let mut encoder = Encoder::new(
        context.clone(),
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

    let context = helpers::create_context();
    let decoder = Decoder::new(
        context.clone(),
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

    // エンコーダーとデコーダーは同じ Context を共有する
    let context = helpers::create_context();

    // エンコードハンドラーは受け取った Buffer のメソッドを呼ぶ
    let encoded = Arc::new(Mutex::new(Vec::new()));
    let encode_observations = Arc::new(Mutex::new(Vec::new()));
    let e = Arc::clone(&encoded);
    let eo = Arc::clone(&encode_observations);
    let mut encoder = Encoder::new(
        context.clone(),
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
        context.clone(),
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
/// `Component` は公開 API から安全に構築できないため、ここでは `Surface` / `Buffer` /
/// `Plane` / `PropertyStorage` を対象にする。
#[test]
fn test_surface_into_raw_and_from_raw() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    let context = helpers::create_context();
    let encoder = Encoder::new(
        context.clone(),
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");

    // 参照の所有権を呼び出し元へ移す
    let ptr = surface.into_raw();
    assert!(!ptr.is_null(), "into_raw() が null を返した");

    // 生ポインタからハンドルを再構築できる
    let surface =
        unsafe { Surface::from_raw(ptr, context.clone()) }.expect("Surface の再構築に失敗");
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

    let context = helpers::create_context();
    let decoder = Decoder::new(
        context.clone(),
        h264_test_decoder_config(),
        FnDecodeHandler::<()>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");
    let buffer = decoder
        .alloc_buffer(BUFFER_SIZE)
        .expect("Buffer の確保に失敗");

    let ptr = buffer.into_raw();
    assert!(!ptr.is_null(), "into_raw() が null を返した");

    let buffer = unsafe { Buffer::from_raw(ptr, context.clone()) }.expect("Buffer の再構築に失敗");
    assert_eq!(buffer.get_size(), BUFFER_SIZE, "サイズが一致しない");
    assert!(!buffer.get_native().is_null(), "バッファが null");

    // Buffer を Decoder より先に破棄する
    drop(buffer);
    drop(decoder);
}

/// `Plane` の `into_raw()` と `from_raw()` を確認する
#[test]
fn test_plane_into_raw_and_from_raw() {
    let context = helpers::create_context();
    let encoder = Encoder::new(
        context.clone(),
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

    let plane = unsafe { Plane::from_raw(ptr, context.clone()) }.expect("Plane の再構築に失敗");
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
    let context = helpers::create_context();
    let encoder = Encoder::new(
        context.clone(),
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

    let property_storage = unsafe { PropertyStorage::from_raw(ptr, context.clone()) }
        .expect("PropertyStorage の再構築に失敗");
    property_storage
        .set_property_int64("AmfRustIntoRawTestProperty", 1)
        .expect("プロパティの設定に失敗");

    // PropertyStorage を Surface より先に、Surface を Encoder より先に破棄する
    drop(property_storage);
    drop(surface);
    drop(encoder);
}

// ---------------------------------------------------------------------------
// Context の作成と寿命のテスト
// ---------------------------------------------------------------------------

/// 複数の Context を作成できることを確認する
#[test]
fn test_multiple_contexts_can_be_created() {
    let context_a = helpers::create_context();
    let context_b = helpers::create_context();

    assert_ne!(
        context_a, context_b,
        "別々に作成した Context が等しいと判定された"
    );
    let cloned_context = context_a.clone();
    assert_eq!(
        context_a, cloned_context,
        "Clone した Context が元の Context と等しくない"
    );

    let surface_a = context_a
        .alloc_surface(
            shiguredo_amf::ffi::AMF_MEMORY_TYPE::AMF_MEMORY_HOST,
            shiguredo_amf::ffi::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12,
            320,
            240,
        )
        .expect("Context A の Surface の確保に失敗");
    let surface_b = context_b
        .alloc_surface(
            shiguredo_amf::ffi::AMF_MEMORY_TYPE::AMF_MEMORY_HOST,
            shiguredo_amf::ffi::AMF_SURFACE_FORMAT::AMF_SURFACE_NV12,
            320,
            240,
        )
        .expect("Context B の Surface の確保に失敗");

    assert_eq!(
        surface_a
            .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
            .expect("Context A の Surface の Y プレーンの取得に失敗")
            .get_width(),
        320,
        "Context A の Surface の幅が一致しない"
    );
    assert_eq!(
        surface_b
            .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
            .expect("Context B の Surface の Y プレーンの取得に失敗")
            .get_width(),
        320,
        "Context B の Surface の幅が一致しない"
    );

    assert_eq!(
        surface_a.context(),
        context_a,
        "Surface A の Context が確保元と一致しない"
    );
    assert_ne!(
        surface_a.context(),
        context_b,
        "Surface A の Context が別の Context と等しいと判定された"
    );
    assert_eq!(
        surface_b.context(),
        context_b,
        "Surface B の Context が確保元と一致しない"
    );
}

/// 各ハンドルが保持する Context を公開 API から取得できることを確認する
#[test]
fn test_handles_expose_context() {
    let context = helpers::create_context();

    let encoder = Encoder::new(
        context.clone(),
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    assert_eq!(
        encoder.context(),
        context,
        "Encoder の Context が作成時の Context と一致しない"
    );
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    assert_eq!(
        surface.context(),
        context,
        "Surface の Context が確保元と一致しない"
    );
    let plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");
    assert_eq!(
        plane.context(),
        context,
        "Plane の Context が取得元と一致しない"
    );
    let property_storage = surface
        .property_storage()
        .expect("PropertyStorage の取得に失敗");
    assert_eq!(
        property_storage.context(),
        context,
        "PropertyStorage の Context が取得元と一致しない"
    );

    let decoder = Decoder::new(
        context.clone(),
        h264_test_decoder_config(),
        FnDecodeHandler::<()>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");
    assert_eq!(
        decoder.context(),
        context,
        "Decoder の Context が作成時の Context と一致しない"
    );
    let buffer = decoder.alloc_buffer(4096).expect("Buffer の確保に失敗");
    assert_eq!(
        buffer.context(),
        context,
        "Buffer の Context が確保元と一致しない"
    );

    drop(property_storage);
    drop(plane);
    drop(surface);
    drop(decoder);
    drop(encoder);
}

/// Context の Clone とハンドルが AMFContext の参照カウントを増やすことを確認する
#[test]
fn test_context_clone_and_handles_increment_ref_count() {
    let context = helpers::create_context();
    assert_eq!(
        context_ref_count(&context),
        1,
        "Context 作成直後の参照カウントが 1 でない"
    );

    let cloned = context.clone();
    assert_eq!(
        context_ref_count(&context),
        2,
        "Context の Clone で参照カウントが増えていない"
    );
    drop(cloned);
    assert_eq!(
        context_ref_count(&context),
        1,
        "Context の Clone を drop しても参照カウントが減っていない"
    );

    let surface = context
        .alloc_surface(
            AMF_MEMORY_TYPE::AMF_MEMORY_HOST,
            AMF_SURFACE_FORMAT::AMF_SURFACE_NV12,
            320,
            240,
        )
        .expect("Surface の確保に失敗");
    assert_eq!(
        context_ref_count(&context),
        2,
        "Surface が Context の参照を保持していない"
    );
    drop(surface);
    assert_eq!(
        context_ref_count(&context),
        1,
        "Surface を drop しても Context の参照カウントが減っていない"
    );

    let buffer = context
        .alloc_buffer(AMF_MEMORY_TYPE::AMF_MEMORY_HOST, 4096)
        .expect("Buffer の確保に失敗");
    assert_eq!(
        context_ref_count(&context),
        2,
        "Buffer が Context の参照を保持していない"
    );
    drop(buffer);
    assert_eq!(
        context_ref_count(&context),
        1,
        "Buffer を drop しても Context の参照カウントが減っていない"
    );
}

/// 利用者が保持する Context を drop しても、Vulkan Surface が Context を保持して convert できることを確認する
#[test]
fn test_vulkan_surface_convert_after_context_dropped() {
    let context = helpers::create_context();
    let surface = context
        .alloc_surface(
            AMF_MEMORY_TYPE::AMF_MEMORY_VULKAN,
            AMF_SURFACE_FORMAT::AMF_SURFACE_NV12,
            320,
            240,
        )
        .expect("Vulkan Surface の確保に失敗");

    drop(context);

    assert_eq!(
        surface.convert(AMF_MEMORY_TYPE::AMF_MEMORY_HOST),
        AMF_RESULT::AMF_OK,
        "Context を drop した後に Vulkan Surface の convert が失敗した"
    );
}

/// Encoder を破棄し、利用者が保持していた Context も破棄した後で Surface を利用できることを確認する
#[test]
fn test_surface_outlives_encoder_and_context_handle() {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    let context = helpers::create_context();
    let retained_context = context.clone();
    let encoder = Encoder::new(
        context.clone(),
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
    fill_nv12_surface(&surface, WIDTH, HEIGHT);

    // Encoder と利用者が保持していた Context を破棄する
    drop(encoder);
    drop(context);

    // 保持しておいた Context があるため、Surface を引き続き利用できる
    let y_plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");
    assert_eq!(y_plane.get_width(), WIDTH as i32, "幅が一致しない");
    assert_eq!(y_plane.get_height(), HEIGHT as i32, "高さが一致しない");
    drop(y_plane);

    // 保持しておいた Context から、同じ Context の Encoder を作成してエンコードできる
    let mut encoder = Encoder::new(
        retained_context,
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    encoder
        .encode(surface, &EncodeOptions::default(), ())
        .expect("別の Encoder でのエンコードに失敗");
    encoder.finish().expect("finish に失敗");
}

/// Decoder を破棄し、利用者が保持していた Context も破棄した後で Buffer を利用できることを確認する
#[test]
fn test_buffer_outlives_decoder_and_context_handle() {
    const BUFFER_SIZE: usize = 4096;

    let context = helpers::create_context();
    let decoder = Decoder::new(
        context.clone(),
        h264_test_decoder_config(),
        FnDecodeHandler::<()>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");
    let buffer = decoder
        .alloc_buffer(BUFFER_SIZE)
        .expect("Buffer の確保に失敗");

    drop(decoder);
    drop(context);

    // Buffer が Context を保持しているため、引き続きアクセスできる
    assert_eq!(buffer.get_size(), BUFFER_SIZE, "サイズが一致しない");
    assert!(!buffer.get_native().is_null(), "バッファが null");
}

/// 利用者が保持する Context がすべてなくなっても、Surface が Context を保持することを確認する
#[test]
fn test_surface_keeps_context_alive() {
    let context = helpers::create_context();
    let encoder = Encoder::new(
        context.clone(),
        h264_cqp_test_encoder_config(),
        FnEncodeHandler::<()>::new(|_| {}),
    )
    .expect("エンコーダーの作成に失敗");
    let surface = encoder.alloc_surface().expect("Surface の確保に失敗");

    // 利用者が保持する Context をすべて破棄する
    drop(encoder);
    drop(context);

    // Surface が Context を保持しているため、引き続き利用できる
    let y_plane = surface
        .get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)
        .expect("Y プレーンの取得に失敗");
    assert_eq!(y_plane.get_width(), 320, "幅が一致しない");
}

/// 利用者が保持する Context がすべてなくなっても、Buffer が Context を保持することを確認する
#[test]
fn test_buffer_keeps_context_alive() {
    let context = helpers::create_context();
    let decoder = Decoder::new(
        context.clone(),
        h264_test_decoder_config(),
        FnDecodeHandler::<()>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");
    let buffer = decoder.alloc_buffer(4096).expect("Buffer の確保に失敗");

    // 利用者が保持する Context をすべて破棄する
    drop(decoder);
    drop(context);

    // Buffer が Context を保持しているため、引き続き利用できる
    assert_eq!(buffer.get_size(), 4096, "サイズが一致しない");
    assert!(!buffer.get_native().is_null(), "バッファが null");
}

/// 複数の Context を作成し、それぞれに Encoder と Decoder を並列に作成できることを確認する
#[test]
fn test_parallel_encoder_decoder_creation() {
    const THREAD_COUNT: usize = 8;

    let barrier = Arc::new(Barrier::new(THREAD_COUNT));
    std::thread::scope(|scope| {
        for _ in 0..THREAD_COUNT {
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                let context = helpers::create_context();
                let encoder = Encoder::new(
                    context.clone(),
                    h264_cqp_test_encoder_config(),
                    FnEncodeHandler::<()>::new(|_| {}),
                )
                .expect("エンコーダーの作成に失敗");
                let decoder = Decoder::new(
                    context,
                    h264_test_decoder_config(),
                    FnDecodeHandler::<()>::new(|_| {}),
                )
                .expect("デコーダーの作成に失敗");

                drop(decoder);
                drop(encoder);
            });
        }
    });
}

/// 同じ Context を複数スレッドで共有して Encoder / Decoder を作成できることを確認する
#[test]
fn test_shared_context_across_threads() {
    const THREAD_COUNT: usize = 8;

    let context = helpers::create_context();
    let barrier = Arc::new(Barrier::new(THREAD_COUNT));
    std::thread::scope(|scope| {
        for _ in 0..THREAD_COUNT {
            let context = context.clone();
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                let encoder = Encoder::new(
                    context.clone(),
                    h264_cqp_test_encoder_config(),
                    FnEncodeHandler::<()>::new(|_| {}),
                )
                .expect("エンコーダーの作成に失敗");
                let decoder = Decoder::new(
                    context,
                    h264_test_decoder_config(),
                    FnDecodeHandler::<()>::new(|_| {}),
                )
                .expect("デコーダーの作成に失敗");

                drop(decoder);
                drop(encoder);
            });
        }
    });
}
