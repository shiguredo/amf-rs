//! テスト用の共通ヘルパー

use shiguredo_amf::AmfLibrary;
use shiguredo_amf::amf::{Context, Surface};
use shiguredo_amf::{
    CodecConfig, EncoderConfig, FrameFormat, H264EncoderConfig, H264Profile, PlaneType,
    RateControlMode,
};

/// テスト用の Context を作成する
pub fn create_context() -> Context {
    AmfLibrary::instance()
        .create_context()
        .expect("Context の作成に失敗")
}

/// テスト用の H.264 CBR エンコーダー設定を返す
pub fn h264_cbr_test_encoder_config() -> EncoderConfig {
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
    config
}

/// NV12 の Y プレーンと UV プレーンの各行を書き込む
///
/// `write_y` は Y プレーンの行、`write_uv` は UV プレーンの行を受け取る。
pub fn write_nv12_rows(
    surface: &Surface,
    width: usize,
    height: usize,
    mut write_y: impl FnMut(usize, &mut [u8]),
    mut write_uv: impl FnMut(usize, &mut [u8]),
) {
    let mut y_plane = surface
        .get_plane(PlaneType::Y)
        .expect("Y プレーンの取得に失敗");
    let y_hpitch = y_plane.get_hpitch() as usize;
    let y_data = y_plane
        .as_mut_slice()
        .expect("Y プレーンのスライス取得に失敗");
    for row in 0..height {
        let start = row * y_hpitch;
        write_y(row, &mut y_data[start..start + width]);
    }

    let mut uv_plane = surface
        .get_plane(PlaneType::Uv)
        .expect("UV プレーンの取得に失敗");
    let uv_hpitch = uv_plane.get_hpitch() as usize;
    let uv_data = uv_plane
        .as_mut_slice()
        .expect("UV プレーンのスライス取得に失敗");
    for row in 0..height / 2 {
        let start = row * uv_hpitch;
        write_uv(row, &mut uv_data[start..start + width]);
    }
}

/// ダミー NV12 フレームを生成する
///
/// Y プレーンはフレーム番号に応じたグラデーション、UV プレーンは 128 固定。
pub fn generate_dummy_nv12(width: usize, height: usize, frame_index: usize) -> Vec<u8> {
    let y_size = width * height;
    let uv_size = y_size / 2;
    let mut data = vec![0u8; y_size + uv_size];

    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = ((x + y + frame_index * 7) % 256) as u8;
        }
    }
    for i in 0..uv_size {
        data[y_size + i] = 128;
    }

    data
}

/// NV12 データを Surface にコピーする
pub fn copy_nv12_to_surface(surface: &Surface, data: &[u8], width: usize, height: usize) {
    let y_size = width * height;
    write_nv12_rows(
        surface,
        width,
        height,
        |row, y_row| {
            let start = row * width;
            y_row.copy_from_slice(&data[start..start + width]);
        },
        |row, uv_row| {
            let start = y_size + row * width;
            uv_row.copy_from_slice(&data[start..start + width]);
        },
    );
}
