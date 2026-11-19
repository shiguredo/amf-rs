//! `Decoder` のデコード結果とユーザーデータの対応に関するテスト
//!
//! このファイルのテストは実 GPU と AMF ランタイムを必要とする。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use shiguredo_amf::amf::{Context, Surface};
use shiguredo_amf::{
    CodecConfig, DecodedFrame, Decoder, DecoderCodec, DecoderConfig, EncodeOptions, Encoder,
    EncoderConfig, Error, FnDecodeHandler, FnEncodeHandler, FrameFormat, H264EncoderConfig,
    PlaneType, RateControlMode, frame_type,
};

#[expect(dead_code, reason = "test_decode では一部のヘルパーだけを使う")]
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
fn input_pts(index: usize) -> i64 {
    1_000_000 + index as i64 * 7
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
        let mut buffer = decoder
            .alloc_buffer(access_unit.len())
            .expect("Buffer の確保に失敗");
        // テスト用の入力 Buffer は他から共有されていないため、AMF に渡す前に
        // ビットストリームを書き込んでよい。
        let buffer_data = unsafe { buffer.as_mut_slice() }.expect("Buffer のスライス取得に失敗");
        assert!(
            buffer_data.len() >= access_unit.len(),
            "Buffer のサイズ {} がアクセスユニットのサイズ {} より小さい",
            buffer_data.len(),
            access_unit.len()
        );
        buffer_data[..access_unit.len()].copy_from_slice(access_unit);

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

/// 入力を 1 つも投入せずに finish した場合、通知がなく完了することを確認する
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

    // 入力がない場合は Drain を呼ばず、Stop でワーカーを終了する。
    decoder.finish().expect("入力なしの finish に失敗");
    drop(decoder);

    let results = Arc::try_unwrap(results)
        .expect("結果一覧の Arc の参照が残っている")
        .into_inner()
        .expect("結果一覧のロックに失敗");
    assert!(results.is_empty(), "入力なしでデコード結果が通知された");
}

/// 最初の SubmitInput がハードエラーになっても finish がハングしないことを確認する
#[test]
fn test_finish_after_failed_first_submit_does_not_hang() {
    let context = create_context();
    let (result_tx, result_rx) = std::sync::mpsc::channel();

    let handle = std::thread::spawn(move || {
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

        // H.264 として解釈できない 1 バイトの入力を最初に投入し、
        // AMF が入力を受理しない状態を作る。
        let mut buffer = decoder.alloc_buffer(1).expect("Buffer の確保に失敗");
        unsafe { buffer.as_mut_slice() }
            .expect("Buffer のスライス取得に失敗")
            .copy_from_slice(&[0]);
        let error = decoder
            .decode(buffer, 0)
            .expect_err("不正なバッファの decode が成功した");
        assert!(
            matches!(
                &error,
                Error::Amf(amf) if amf.operation() == "AMFComponent::SubmitInput"
            ),
            "AMFComponent::SubmitInput の AMF エラーではない: {error}"
        );

        let finish_result = decoder.finish();
        drop(decoder);

        let results = Arc::try_unwrap(results)
            .expect("結果一覧の Arc の参照が残っている")
            .into_inner()
            .expect("結果一覧のロックに失敗");
        result_tx
            .send((finish_result, results))
            .expect("結果の送信に失敗");
    });

    // 入力を受理していないため finish は Drain を待たずに Stop で終了する。
    // 退行すると finish が永久に戻らないため、watchdog で検出する。
    let (finish_result, results) = result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("入力未受理の finish がハングした");
    finish_result.expect("入力未受理の finish に失敗");
    assert!(
        results.is_empty(),
        "入力未受理の finish でデコード結果が通知された"
    );

    handle.join().expect("テストスレッドが panic した");
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

    let mut buffer = decoder
        .alloc_buffer(data.len())
        .expect("Buffer の確保に失敗");
    // テスト用の入力 Buffer は他から共有されていないため、AMF に渡す前に
    // ビットストリームを書き込んでよい。
    unsafe { buffer.as_mut_slice() }
        .expect("Buffer のスライス取得に失敗")
        .copy_from_slice(&data);

    let error = decoder
        .decode(buffer, 0)
        .expect_err("複数フレームを含むバッファの decode が成功した");
    assert!(
        matches!(&error, Error::InvalidArgument(_)),
        "複数フレームを含むバッファの decode が InvalidArgument ではない: {error}"
    );
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
        matches!(&error, Error::InvalidState(_)),
        "finish 後の decode が InvalidState ではない: {error}"
    );
    assert!(
        error.to_string().contains("already finished"),
        "finish 後の decode のエラーメッセージが想定と異なる: {error}"
    );
}

/// finish を呼ばずに Decoder を drop すると、保留中の入力が通知されないことを確認する
#[test]
fn test_drop_decoder_without_finish_does_not_notify_pending_input() {
    let access_units = split_access_units(H264_BFRAME_STREAM, is_h264_vcl);
    // パラメータセットだけではデコード出力が生成されないため、drop による通知の
    // 有無を確実に確認できる。
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

    let mut buffer = decoder
        .alloc_buffer(parameter_sets.len())
        .expect("Buffer の確保に失敗");
    unsafe { buffer.as_mut_slice() }
        .expect("Buffer のスライス取得に失敗")
        .copy_from_slice(&parameter_sets);
    decoder.decode(buffer, 0).expect("デコードの投入に失敗");

    let started = Instant::now();
    drop(decoder);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "Decoder の drop に時間がかかりすぎた: {:?}",
        started.elapsed()
    );

    let results = Arc::try_unwrap(results)
        .expect("結果一覧の Arc の参照が残っている")
        .into_inner()
        .expect("結果一覧のロックに失敗");
    assert!(
        results.is_empty(),
        "finish を呼ばずに drop したのに保留中の入力が通知された"
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
        let mut buffer = decoder
            .alloc_buffer(access_unit.len())
            .expect("Buffer の確保に失敗");
        unsafe { buffer.as_mut_slice() }
            .expect("Buffer のスライス取得に失敗")
            .copy_from_slice(access_unit);
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

    let mut buffer = decoder
        .alloc_buffer(parameter_sets.len())
        .expect("Buffer の確保に失敗");
    unsafe { buffer.as_mut_slice() }
        .expect("Buffer のスライス取得に失敗")
        .copy_from_slice(&parameter_sets);
    // 利用者が設定した PTS が、未対応入力のエラーメッセージに使われることを確認する。
    buffer.set_pts(100_000);
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
        matches!(&error, Error::Internal(_)),
        "未対応入力のエラーが Internal ではない: {error}"
    );
    assert!(
        error.to_string().contains("PTS 100000"),
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
        matches!(&error, Error::InvalidState(_)),
        "2 回目の finish が InvalidState ではない: {error}"
    );
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

// ---------------------------------------------------------------------------
// 解像度変化
// ---------------------------------------------------------------------------

/// 最初の解像度 (320x240) のフレーム数
const RESOLUTION_CHANGE_INITIAL_FRAMES: usize = 30;

/// 縮小後の解像度 (256x160) のフレーム数
const RESOLUTION_CHANGE_SMALLER_FRAMES: usize = 15;

/// 拡大後の解像度 (320x240) のフレーム数
const RESOLUTION_CHANGE_LARGER_FRAMES: usize = 15;

/// 指定した解像度の H.264 ストリームをエンコードし、フレームごとのビット列を返す
///
/// 解像度の異なるストリームを連結するため、Buffer ではなくビット列として
/// 取り出す。
fn encode_h264_stream(
    context: &Context,
    width: u32,
    height: u32,
    frame_count: usize,
) -> Vec<Vec<u8>> {
    let mut config = EncoderConfig::new(
        CodecConfig::H264(H264EncoderConfig { profile: None }),
        width,
        height,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1000);

    let results = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&results);
    let mut encoder = Encoder::new(
        context.clone(),
        config,
        FnEncodeHandler::<()>::new(move |result| {
            received
                .lock()
                .expect("エンコード結果を格納する Mutex のロックに失敗")
                .push(result.expect("エンコード結果の取得に失敗"));
        }),
    )
    .expect("エンコーダーの作成に失敗");

    for index in 0..frame_count {
        let surface = encoder.alloc_surface().expect("Surface の確保に失敗");
        let frame = helpers::generate_dummy_nv12(width as usize, height as usize, index);
        helpers::copy_nv12_to_surface(&surface, &frame, width as usize, height as usize);
        encoder
            .encode(
                surface,
                &EncodeOptions {
                    frame_type: frame_type::UNKNOWN,
                },
                (),
            )
            .expect("エンコードに失敗");
    }
    encoder.finish().expect("finish に失敗");
    drop(encoder);

    Arc::try_unwrap(results)
        .expect("エンコード結果の Arc の参照が残っている")
        .into_inner()
        .expect("エンコード結果の Mutex がポイズンされている")
        .into_iter()
        .map(|frame| {
            let (buffer, ()) = frame.into_parts();
            unsafe { buffer.as_slice() }
                .expect("エンコード結果のスライス取得に失敗")
                .to_vec()
        })
        .collect()
}

/// Surface の Y プレーンの幅と高さを返す
fn surface_size(surface: &Surface) -> (usize, usize) {
    let plane = surface
        .get_plane(PlaneType::Y)
        .expect("Y プレーンの取得に失敗");
    (plane.get_width() as usize, plane.get_height() as usize)
}

/// 解像度が 320x240 → 256x160 (縮小) → 320x240 (拡大) と変化する
/// 60 フレームのストリームを欠落・重複なくデコードできることを確認する
#[test]
fn test_decode_continues_after_resolution_change() {
    // 320x240 x 30 + 256x160 x 15 + 320x240 x 15 の 3 本のストリームを
    // 1 本に連結する。
    let encoding_context = create_context();
    let mut access_units = encode_h264_stream(
        &encoding_context,
        320,
        240,
        RESOLUTION_CHANGE_INITIAL_FRAMES,
    );
    access_units.extend(encode_h264_stream(
        &encoding_context,
        256,
        160,
        RESOLUTION_CHANGE_SMALLER_FRAMES,
    ));
    access_units.extend(encode_h264_stream(
        &encoding_context,
        320,
        240,
        RESOLUTION_CHANGE_LARGER_FRAMES,
    ));
    drop(encoding_context);

    let total_frames = RESOLUTION_CHANGE_INITIAL_FRAMES
        + RESOLUTION_CHANGE_SMALLER_FRAMES
        + RESOLUTION_CHANGE_LARGER_FRAMES;
    assert_eq!(
        access_units.len(),
        total_frames,
        "テスト用ストリームのフレーム数が想定と異なる"
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
                .expect("デコード結果を格納する Mutex のロックに失敗")
                .push(result);
        }),
    )
    .expect("デコーダーの作成に失敗");

    for (index, access_unit) in access_units.iter().enumerate() {
        let mut buffer = decoder
            .alloc_buffer(access_unit.len())
            .expect("Buffer の確保に失敗");
        unsafe { buffer.as_mut_slice() }
            .expect("Buffer のスライス取得に失敗")
            .copy_from_slice(access_unit);
        buffer.set_pts(input_pts(index));
        decoder
            .decode(buffer, index)
            .unwrap_or_else(|error| panic!("フレーム {index} の投入に失敗: {error}"));
    }

    decoder.finish().expect("finish に失敗");
    drop(decoder);

    let results = Arc::try_unwrap(results)
        .expect("デコード結果の Arc の参照が残っている")
        .into_inner()
        .expect("デコード結果の Mutex がポイズンされている");
    let summaries = results
        .iter()
        .map(|result| match result {
            Ok(frame) => format!(
                "ok: user_data={}, pts={}",
                frame.user_data(),
                frame.surface().get_pts()
            ),
            Err(error) => format!("err: {error}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results.len(),
        total_frames,
        "通知されたフレーム数が一致しない: {summaries:?}"
    );

    let mut order = Vec::new();
    for (position, result) in results.into_iter().enumerate() {
        let frame =
            result.unwrap_or_else(|error| panic!("出力 {position} のデコードに失敗: {error}"));
        let (surface, index) = frame.into_parts();
        assert!(
            index < total_frames,
            "未知の user_data {index} が通知された"
        );
        order.push(index);
        assert_eq!(
            surface.get_pts(),
            input_pts(index),
            "フレーム {index} の PTS が入力時の値と一致しない"
        );
        let expected = if index < RESOLUTION_CHANGE_INITIAL_FRAMES {
            (320, 240)
        } else if index < RESOLUTION_CHANGE_INITIAL_FRAMES + RESOLUTION_CHANGE_SMALLER_FRAMES {
            (256, 160)
        } else {
            (320, 240)
        };
        assert_eq!(
            surface_size(&surface),
            expected,
            "フレーム {index} の解像度が想定と異なる"
        );
    }
    assert_eq!(
        order,
        (0..total_frames).collect::<Vec<_>>(),
        "解像度変化を挟んでも出力順が入力順と一致しない"
    );
}
