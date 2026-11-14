//! `Decoder` のデコード結果とユーザーデータの対応に関するテスト
//!
//! このファイルのテストは実 GPU と AMF ランタイムを必要とする。

use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use shiguredo_amf::ffi::amf_pts;
use shiguredo_amf::{DecodedFrame, Decoder, DecoderCodec, DecoderConfig, Error, FnDecodeHandler};

#[expect(dead_code, reason = "test_decode では create_context だけを使う")]
mod helpers;

use helpers::create_context;

/// B フレームを含む H.264 の Annex B ビットストリーム
const H264_BFRAME_STREAM: &[u8] = include_bytes!("data/red-bframe-320x320-h264.h264");

/// B フレームを含む HEVC の Annex B ビットストリーム
const HEVC_BFRAME_STREAM: &[u8] = include_bytes!("data/cars-320x240.h265");

/// H.264 フィクスチャの提示順を入力インデックスで表したもの
///
/// 元の MP4 の ctts から求めた提示順であり、AMF の出力 PTS には依存しない。
const H264_PRESENTATION_ORDER: [usize; 50] = [
    0, 2, 3, 1, 5, 6, 4, 8, 9, 7, 11, 12, 10, 14, 15, 13, 17, 18, 16, 20, 21, 19, 23, 24, 22, 26,
    27, 25, 29, 30, 28, 32, 33, 31, 35, 36, 34, 38, 39, 37, 41, 42, 40, 44, 45, 43, 47, 48, 46, 49,
];

/// 入力フレームに設定する PTS を返す
///
/// 入力 PTS が復元されることを確認できるよう、フレームごとに異なる値を返す。
fn input_pts(index: usize) -> amf_pts {
    1_000_000 + index as amf_pts * 7
}

/// Annex B 形式のビットストリームをアクセスユニット単位に分割する
///
/// `is_vcl` は NAL ユニットが VCL (スライス) かどうかを判定する。
/// VCL NAL が現れるたびに、それまでの非 VCL NAL を含めて 1 アクセスユニットとして
/// 切り出す。テスト用のビットストリームは 1 アクセスユニットに 1 つの VCL NAL を
/// 含む前提である。
fn split_access_units(data: &[u8], is_vcl: impl Fn(u8) -> bool) -> Vec<Vec<u8>> {
    let mut access_units = Vec::new();
    let mut current = Vec::new();
    let mut position = 0;
    while position + 3 <= data.len() {
        let start_code_len = if data[position..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if data[position..].starts_with(&[0, 0, 1]) {
            3
        } else {
            position += 1;
            continue;
        };

        let nal_start = position + start_code_len;
        if nal_start >= data.len() {
            break;
        }
        let mut nal_end = data.len();
        let mut next = nal_start;
        while next + 3 <= data.len() {
            if data[next..].starts_with(&[0, 0, 0, 1]) || data[next..].starts_with(&[0, 0, 1]) {
                nal_end = next;
                break;
            }
            next += 1;
        }

        current.extend_from_slice(&data[position..nal_end]);

        // NAL ユニットヘッダーの 1 バイト目から NAL ユニット種別を取り出す。
        // H.264 と HEVC でビット位置が異なるため、判定は呼び出し元に任せる。
        if is_vcl(data[nal_start]) {
            access_units.push(std::mem::take(&mut current));
        }

        position = nal_end;
    }

    // VCL NAL より後ろに残った非 VCL NAL は、どのアクセスユニットにも含めない。
    // テスト用の HEVC ビットストリームでは末尾の EOS_NUT がこれに該当する。
    access_units
}

/// Annex B 形式のデータから VCL 以外の NAL ユニットだけを抜き出す
fn without_vcl_nals(data: &[u8], is_vcl: impl Fn(u8) -> bool) -> Vec<u8> {
    let mut output = Vec::new();
    let mut position = 0;
    while position + 3 <= data.len() {
        let start_code_len = if data[position..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if data[position..].starts_with(&[0, 0, 1]) {
            3
        } else {
            position += 1;
            continue;
        };

        let nal_start = position + start_code_len;
        if nal_start >= data.len() {
            break;
        }
        let mut nal_end = data.len();
        let mut next = nal_start;
        while next + 3 <= data.len() {
            if data[next..].starts_with(&[0, 0, 0, 1]) || data[next..].starts_with(&[0, 0, 1]) {
                nal_end = next;
                break;
            }
            next += 1;
        }

        if !is_vcl(data[nal_start]) {
            output.extend_from_slice(&data[position..nal_end]);
        }
        position = nal_end;
    }
    output
}

/// H.264 の NAL ユニット種別が VCL かどうかを返す
fn is_h264_vcl(nal_header: u8) -> bool {
    matches!(nal_header & 0x1f, 1..=5)
}

/// HEVC の NAL ユニット種別が VCL かどうかを返す
fn is_hevc_vcl(nal_header: u8) -> bool {
    // HEVC の NAL ユニット種別は、NAL ユニットヘッダーの 1 バイト目の
    // bit 1 から bit 6 までである。
    ((nal_header >> 1) & 0x3f) <= 31
}

/// 各アクセスユニットを個別の Buffer でデコードし、入力順のインデックスを
/// ユーザーデータとして渡した結果を返す
fn decode_access_units(
    codec: DecoderCodec,
    access_units: &[Vec<u8>],
) -> Vec<Result<DecodedFrame<usize>, Error>> {
    let context = create_context();
    let results = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&results);

    let mut decoder = Decoder::new(
        context,
        DecoderConfig { codec },
        FnDecodeHandler::<usize>::new(move |result| {
            received
                .lock()
                .expect("結果一覧のロックに失敗")
                .push(result);
        }),
    )
    .expect("デコーダーの作成に失敗");

    for (index, access_unit) in access_units.iter().enumerate() {
        let buffer = decoder
            .alloc_buffer(access_unit.len())
            .expect("Buffer の確保に失敗");
        let native = buffer.get_native() as *mut u8;
        assert!(!native.is_null(), "Buffer の native ポインターが null");
        assert!(
            buffer.get_size() >= access_unit.len(),
            "Buffer のサイズ {} がアクセスユニットのサイズ {} より小さい",
            buffer.get_size(),
            access_unit.len()
        );
        // テスト用の入力 Buffer は他から共有されていないため、AMF に渡す前に
        // ビットストリームを書き込んでよい。
        unsafe {
            ptr::copy_nonoverlapping(access_unit.as_ptr(), native, access_unit.len());
        }

        // 入力時の PTS がデコード結果に保持されることを確認できるよう、内部の
        // 対応付け用 ID とは異なる値を設定する。
        buffer.set_pts(input_pts(index));
        decoder
            .decode(buffer, index)
            .unwrap_or_else(|error| panic!("アクセスユニット {index} の投入に失敗: {error}"));
    }

    decoder.finish().expect("finish に失敗");
    drop(decoder);

    Arc::try_unwrap(results)
        .expect("結果一覧の Arc の参照が残っている")
        .into_inner()
        .expect("結果一覧のロックに失敗")
}

/// 出力フレームの PTS が入力時の PTS と一致し、出力順が入力順と異なることを確認する
///
/// B フレームを含むストリームでは出力が提示順になるため、PTS の並びは入力順と
/// 一致しない。各出力の `user_data` がその PTS に対応する入力のインデックスで
/// あれば、入力と出力の対応付けが正しいと判断できる。
fn assert_user_data_follows_pts(
    results: Vec<Result<DecodedFrame<usize>, Error>>,
    expected_frames: usize,
) -> Vec<usize> {
    let summaries = results
        .iter()
        .map(|result| match result {
            Ok(frame) => format!("ok: pts={}", frame.surface().get_pts()),
            Err(error) => format!("err: {error}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results.len(),
        expected_frames,
        "デコードされたフレーム数が一致しない: {summaries:?}"
    );

    let mut output_pts = Vec::new();
    let mut user_data_order = Vec::new();
    for (index, result) in results.into_iter().enumerate() {
        let frame = result.unwrap_or_else(|error| panic!("出力 {index} のデコードに失敗: {error}"));
        let (surface, user_data) = frame.into_parts();
        let pts = surface.get_pts();
        user_data_order.push(user_data);
        assert_eq!(
            pts,
            input_pts(user_data),
            "出力 {index} の user_data ({user_data}) と PTS ({pts}) が対応しない"
        );
        output_pts.push(pts);
    }

    let mut sorted = output_pts.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        (0..expected_frames).map(input_pts).collect::<Vec<_>>(),
        "出力に重複または欠落した PTS がある"
    );

    assert!(
        output_pts.windows(2).any(|pair| pair[0] > pair[1]),
        "B フレームを含むストリームの出力 PTS が入力順のままである: {output_pts:?}"
    );

    user_data_order
}

/// 入力を 1 つも投入せずに finish しても、待機せずに完了することを確認する
#[test]
fn test_decoder_finish_without_input() {
    let context = create_context();
    let results = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&results);

    let mut decoder = Decoder::new(
        context,
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<usize>::new(move |result| {
            received
                .lock()
                .expect("結果一覧のロックに失敗")
                .push(result);
        }),
    )
    .expect("デコーダーの作成に失敗");

    // Drain 後に出力がない状態でも、ワーカーは EOF または出力なしを検出して完了する。
    let started = Instant::now();
    decoder.finish().expect("入力なしの finish に失敗");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "入力なしの finish に時間がかかりすぎた: {:?}",
        started.elapsed()
    );
    drop(decoder);

    let results = Arc::try_unwrap(results)
        .expect("結果一覧の Arc の参照が残っている")
        .into_inner()
        .expect("結果一覧のロックに失敗");
    assert!(results.is_empty(), "入力なしでデコード結果が通知された");
}

/// 複数フレームを含むバッファを渡すと、AMF_REPEAT を検出してエラーになることを確認する
#[test]
fn test_decode_rejects_multi_frame_buffer() {
    let access_units = split_access_units(H264_BFRAME_STREAM, is_h264_vcl);
    assert!(
        access_units.len() >= 2,
        "テスト用ビットストリームから複数のアクセスユニットが得られない"
    );

    // AMF_REPEAT が返る程度の複数フレームを 1 つの Buffer に連結する。
    let mut data = Vec::new();
    for access_unit in access_units.iter().take(10) {
        data.extend_from_slice(access_unit);
    }

    let results = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&results);
    let context = create_context();
    let mut decoder = Decoder::new(
        context,
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<usize>::new(move |result| {
            received
                .lock()
                .expect("結果一覧のロックに失敗")
                .push(result);
        }),
    )
    .expect("デコーダーの作成に失敗");

    let buffer = decoder
        .alloc_buffer(data.len())
        .expect("Buffer の確保に失敗");
    // テスト用の入力 Buffer は他から共有されていないため、AMF に渡す前に
    // ビットストリームを書き込んでよい。
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(), buffer.get_native() as *mut u8, data.len());
    }

    let error = decoder
        .decode(buffer, 0)
        .expect_err("複数フレームを含むバッファの decode が成功した");
    assert!(
        error.to_string().contains("multiple frames"),
        "複数フレームを含むバッファのエラーメッセージが想定と異なる: {error}"
    );

    // decode が失敗しても、finish でデコーダー内に残った出力はエラーとして通知される。
    decoder.finish().expect("finish に失敗");
    drop(decoder);

    let results = Arc::try_unwrap(results)
        .expect("結果一覧の Arc の参照が残っている")
        .into_inner()
        .expect("結果一覧のロックに失敗");
    assert!(!results.is_empty(), "未対応出力のエラーが通知されなかった");
    assert!(
        results.iter().all(|result| result.is_err()),
        "未対応出力が成功として通知された"
    );
}

/// finish 後に decode を呼ぶとエラーになることを確認する
#[test]
fn test_decode_after_finish_is_rejected() {
    let context = create_context();
    let mut decoder = Decoder::new(
        context,
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<usize>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");

    decoder.finish().expect("finish に失敗");

    // finish 済みの確認は他の入力検証より前に行われるため、Buffer の中身は空でもよい。
    let buffer = decoder.alloc_buffer(1).expect("Buffer の確保に失敗");
    let error = decoder
        .decode(buffer, 0)
        .expect_err("finish 後の decode が成功した");
    assert!(
        error.to_string().contains("already finished"),
        "finish 後の decode のエラーメッセージが想定と異なる: {error}"
    );
}

/// finish を呼ばずに Decoder を drop しても、ワーカーの停止が完了することを確認する
#[test]
fn test_drop_decoder_without_finish() {
    let access_units = split_access_units(H264_BFRAME_STREAM, is_h264_vcl);
    let access_unit = &access_units[0];

    let mut decoder = Decoder::new(
        create_context(),
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<usize>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");

    let buffer = decoder
        .alloc_buffer(access_unit.len())
        .expect("Buffer の確保に失敗");
    unsafe {
        ptr::copy_nonoverlapping(
            access_unit.as_ptr(),
            buffer.get_native() as *mut u8,
            access_unit.len(),
        );
    }
    decoder.decode(buffer, 0).expect("デコードの投入に失敗");

    let started = Instant::now();
    drop(decoder);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "Decoder の drop に時間がかかりすぎた: {:?}",
        started.elapsed()
    );
}

/// 重複した入力 PTS と PTS 0 でも、フレームごとに正しく対応付けられることを確認する
#[test]
fn test_decode_preserves_duplicate_zero_pts() {
    let access_units = split_access_units(H264_BFRAME_STREAM, is_h264_vcl);
    let frame_count = 10;
    assert!(
        access_units.len() >= frame_count,
        "テスト用ビットストリームのアクセスユニットが足りない"
    );

    let results = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&results);
    let mut decoder = Decoder::new(
        create_context(),
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<usize>::new(move |result| {
            received
                .lock()
                .expect("結果一覧のロックに失敗")
                .push(result);
        }),
    )
    .expect("デコーダーの作成に失敗");

    for (index, access_unit) in access_units.iter().take(frame_count).enumerate() {
        let buffer = decoder
            .alloc_buffer(access_unit.len())
            .expect("Buffer の確保に失敗");
        unsafe {
            ptr::copy_nonoverlapping(
                access_unit.as_ptr(),
                buffer.get_native() as *mut u8,
                access_unit.len(),
            );
        }
        // すべての入力に同じ PTS 0 を設定する。PTS が重複しても内部の
        // 対応付け用 ID で区別できることを確認する。
        buffer.set_pts(0);
        decoder
            .decode(buffer, index)
            .unwrap_or_else(|error| panic!("アクセスユニット {index} の投入に失敗: {error}"));
    }

    decoder.finish().expect("finish に失敗");
    drop(decoder);

    let results = Arc::try_unwrap(results)
        .expect("結果一覧の Arc の参照が残っている")
        .into_inner()
        .expect("結果一覧のロックに失敗");
    assert_eq!(results.len(), frame_count, "デコード結果の件数が一致しない");

    let mut user_data_order = Vec::new();
    for result in results {
        let frame = result.expect("デコードに失敗した");
        let (surface, user_data) = frame.into_parts();
        assert_eq!(
            surface.get_pts(),
            0,
            "入力 PTS 0 が出力 Surface に保持されていない"
        );
        user_data_order.push(user_data);
    }
    user_data_order.sort_unstable();
    assert_eq!(
        user_data_order,
        (0..frame_count).collect::<Vec<_>>(),
        "重複した PTS で user_data が欠落または重複した"
    );
}

/// SPS/PPS だけを投入した場合、finish で未対応入力のエラーが通知されることを確認する
#[test]
fn test_decode_reports_input_without_output() {
    let access_units = split_access_units(H264_BFRAME_STREAM, is_h264_vcl);
    let parameter_sets = without_vcl_nals(&access_units[0], is_h264_vcl);
    assert!(
        !parameter_sets.is_empty(),
        "先頭のアクセスユニットから SPS/PPS を取得できない"
    );

    let results = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&results);
    let mut decoder = Decoder::new(
        create_context(),
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<usize>::new(move |result| {
            received
                .lock()
                .expect("結果一覧のロックに失敗")
                .push(result);
        }),
    )
    .expect("デコーダーの作成に失敗");

    let buffer = decoder
        .alloc_buffer(parameter_sets.len())
        .expect("Buffer の確保に失敗");
    unsafe {
        ptr::copy_nonoverlapping(
            parameter_sets.as_ptr(),
            buffer.get_native() as *mut u8,
            parameter_sets.len(),
        );
    }
    decoder.decode(buffer, 0).expect("SPS/PPS だけの投入に失敗");

    decoder.finish().expect("finish に失敗");
    drop(decoder);

    let results = Arc::try_unwrap(results)
        .expect("結果一覧の Arc の参照が残っている")
        .into_inner()
        .expect("結果一覧のロックに失敗");
    assert_eq!(results.len(), 1, "未対応入力のエラーが 1 件ではない");
    let error = results
        .into_iter()
        .next()
        .expect("結果が存在しない")
        .expect_err("出力のない入力が成功として通知された");
    assert!(
        error.to_string().contains("did not output a frame"),
        "未対応入力のエラーメッセージが想定と異なる: {error}"
    );
}

/// finish を 2 回呼ぶと 2 回目がエラーになることを確認する
#[test]
fn test_finish_twice_is_rejected() {
    let mut decoder = Decoder::new(
        create_context(),
        DecoderConfig {
            codec: DecoderCodec::H264,
        },
        FnDecodeHandler::<usize>::new(|_| {}),
    )
    .expect("デコーダーの作成に失敗");

    decoder.finish().expect("1 回目の finish に失敗");
    let error = decoder.finish().expect_err("2 回目の finish が成功した");
    assert!(
        error.to_string().contains("already finished"),
        "2 回目の finish のエラーメッセージが想定と異なる: {error}"
    );
}

/// B フレームを含む H.264 ストリームで、user_data が対応するフレームに付くことを確認する
#[test]
fn test_decode_h264_bframes_maps_user_data_by_pts() {
    let access_units = split_access_units(H264_BFRAME_STREAM, is_h264_vcl);
    assert!(
        access_units.len() > 1,
        "テスト用ビットストリームから複数のアクセスユニットが得られない"
    );

    assert_eq!(
        access_units.len(),
        H264_PRESENTATION_ORDER.len(),
        "H.264 ビットストリームのアクセスユニット数が想定と異なる"
    );

    let results = decode_access_units(DecoderCodec::H264, &access_units);
    let user_data_order = assert_user_data_follows_pts(results, access_units.len());
    assert_eq!(
        user_data_order, H264_PRESENTATION_ORDER,
        "user_data の並びが MP4 の ctts から求めた提示順と一致しない"
    );
}

/// B フレームを含む HEVC ストリームで、user_data が対応するフレームに付くことを確認する
#[test]
fn test_decode_hevc_bframes_maps_user_data_by_pts() {
    let access_units = split_access_units(HEVC_BFRAME_STREAM, is_hevc_vcl);
    assert!(
        access_units.len() > 1,
        "テスト用ビットストリームから複数のアクセスユニットが得られない"
    );

    assert_eq!(
        access_units.len(),
        30,
        "HEVC ビットストリームのアクセスユニット数が想定と異なる"
    );

    let results = decode_access_units(DecoderCodec::Hevc, &access_units);
    assert_user_data_follows_pts(results, access_units.len());
}
