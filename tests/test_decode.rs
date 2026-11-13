//! `Decoder` のデコード結果とユーザーデータの対応に関するテスト
//!
//! このファイルのテストは実 GPU と AMF ランタイムを必要とする。

use std::ptr;
use std::sync::{Arc, Mutex};

use shiguredo_amf::amf::Context;
use shiguredo_amf::ffi::amf_pts;
use shiguredo_amf::{
    AmfLibrary, DecodedFrame, Decoder, DecoderCodec, DecoderConfig, Error, FnDecodeHandler,
};

/// テスト用の Context を作成する
fn create_context() -> Context {
    AmfLibrary::instance()
        .create_context()
        .expect("Context の作成に失敗")
}

/// B フレームを含む H.264 の Annex B ビットストリーム
const H264_BFRAME_STREAM: &[u8] = include_bytes!("data/red-bframe-320x320-h264.h264");

/// B フレームを含む HEVC の Annex B ビットストリーム
const HEVC_BFRAME_STREAM: &[u8] = include_bytes!("data/cars-320x240.h265");

/// 入力フレームに設定する PTS を返す
///
/// 内部の対応付け用 ID と値が衝突しないよう、アクセスユニットのインデックスとは
/// 異なる規則で単調増加する値を返す。
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

    // VCL NAL より後ろに付く SEI などの非 VCL NAL は、直前のアクセスユニットに
    // 含める。テスト用のビットストリームは末尾の SEI だけがこれに該当する。
    if !current.is_empty() {
        let last = access_units
            .last_mut()
            .expect("VCL NAL より前に非 VCL NAL しかない");
        last.extend_from_slice(&current);
    }
    access_units
}

/// H.264 の NAL ユニット種別が VCL かどうかを返す
fn is_h264_vcl(nal_header: u8) -> bool {
    matches!(nal_header & 0x1f, 1..=5)
}

/// HEVC の NAL ユニット種別が VCL かどうかを返す
fn is_hevc_vcl(nal_header: u8) -> bool {
    // HEVC の NAL ユニットヘッダーは 2 バイトで、種別は 1 バイト目の上位 6 ビット。
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
) {
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
    for (index, result) in results.into_iter().enumerate() {
        let frame = result.unwrap_or_else(|error| panic!("出力 {index} のデコードに失敗: {error}"));
        let (surface, user_data) = frame.into_parts();
        let pts = surface.get_pts();
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
    decoder.finish().expect("入力なしの finish に失敗");
    drop(decoder);

    let results = Arc::try_unwrap(results)
        .expect("結果一覧の Arc の参照が残っている")
        .into_inner()
        .expect("結果一覧のロックに失敗");
    assert!(results.is_empty(), "入力なしでデコード結果が通知された");
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

    // 解像度などの検証より前に finish 済みであることを確認するため、Buffer の
    // 中身は空でもよい。
    let buffer = decoder.alloc_buffer(1).expect("Buffer の確保に失敗");
    let error = decoder
        .decode(buffer, 0)
        .expect_err("finish 後の decode が成功した");
    assert!(
        error.to_string().contains("already finished"),
        "finish 後の decode のエラーメッセージが想定と異なる: {error}"
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

    let results = decode_access_units(DecoderCodec::H264, &access_units);
    assert_user_data_follows_pts(results, access_units.len());
}

/// B フレームを含む HEVC ストリームで、user_data が対応するフレームに付くことを確認する
#[test]
fn test_decode_hevc_bframes_maps_user_data_by_pts() {
    let access_units = split_access_units(HEVC_BFRAME_STREAM, is_hevc_vcl);
    assert!(
        access_units.len() > 1,
        "テスト用ビットストリームから複数のアクセスユニットが得られない"
    );

    let results = decode_access_units(DecoderCodec::Hevc, &access_units);
    assert_user_data_follows_pts(results, access_units.len());
}
