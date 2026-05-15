//! AMF ハードウェアデコーダー
//!
//! AMD GPU を使ったハードウェアビデオデコードを提供する。
//! H.264/AVC、H.265/HEVC、AV1 コーデックに対応する。

use std::collections::VecDeque;
use std::ptr;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::AmfLibrary;
use crate::amf::{Component, Context, Surface};
use crate::error::{Error, positive_i32_to_usize};
use crate::sys::{
    self, AMF_MEMORY_TYPE, AMF_PLANE_TYPE, AMF_RESULT, AMF_SURFACE_FORMAT, AMFData, AMFSurface,
};

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

/// デコーダーのコーデック種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderCodec {
    H264,
    Hevc,
    Av1,
}

/// デコーダー設定
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    pub codec: DecoderCodec,
}

/// デコード済みフレーム (NV12 形式)
#[derive(Debug)]
pub struct DecodedFrame {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl DecodedFrame {
    /// NV12 フレームデータへの参照
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// フレームデータの所有権を取得する
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// フレーム幅
    pub fn width(&self) -> usize {
        self.width
    }

    /// フレーム高さ
    pub fn height(&self) -> usize {
        self.height
    }
}

// ---------------------------------------------------------------------------
// ワーカースレッド用コマンド
// ---------------------------------------------------------------------------

enum WorkerCommand<T> {
    Submit(T),
    Finish(mpsc::SyncSender<()>),
}

// ---------------------------------------------------------------------------
// デコーダー実装
// ---------------------------------------------------------------------------

/// AMF ハードウェアデコーダー
pub struct Decoder<T: Send + 'static> {
    component: Component,
    context: Context,
    cmd_tx: Option<mpsc::Sender<WorkerCommand<T>>>,
    poll_thread: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> Decoder<T> {
    /// デコーダーを作成する
    ///
    /// `callback` はデコード完了時にワーカースレッドから呼び出される。
    /// 第 1 引数はデコード済みフレーム、第 2 引数は `decode()` に渡された `T` の値。
    pub fn new(
        config: DecoderConfig,
        callback: impl FnMut(DecodedFrame, T) + Send + 'static,
    ) -> Result<Self, Error> {
        let lib = AmfLibrary::instance();
        let context = lib.create_context()?;

        unsafe { context.init_vulkan(ptr::null_mut()) }?;

        let component_id = match config.codec {
            DecoderCodec::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
            DecoderCodec::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
            DecoderCodec::Av1 => sys::str::AMFVideoDecoderHW_AV1,
        };

        let component = lib.create_component(&context, component_id)?;

        // デコーダーを初期化する (解像度は 0,0 でストリームから自動検出)
        let result = component.init(AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, 0, 0);
        Error::check(result, "AMFComponent::Init")?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand<T>>();
        let worker_component = component.clone();
        let poll_thread = std::thread::Builder::new()
            .name("amf-decoder-worker".into())
            .spawn(move || {
                worker(worker_component, callback, cmd_rx);
            })
            .map_err(|e| {
                Error::new_custom(
                    "Decoder::new",
                    &format!("failed to spawn worker thread: {e}"),
                )
            })?;

        Ok(Self {
            component,
            context,
            cmd_tx: Some(cmd_tx),
            poll_thread: Some(poll_thread),
        })
    }

    /// ビットストリームをデコードする
    ///
    /// `user_data` はデコード完了時にコールバックへ渡される。
    pub fn decode(&mut self, data: &[u8], user_data: T) -> Result<(), Error> {
        if data.is_empty() {
            return Ok(());
        }

        log::debug!("Decoder::decode: data size={}", data.len());

        let buffer = self
            .context
            .alloc_buffer(AMF_MEMORY_TYPE::AMF_MEMORY_HOST, data.len())?;

        let buf_native = buffer.get_native() as *mut u8;
        if buf_native.is_null() {
            return Err(Error::new_custom(
                "Decoder::decode",
                "buffer native pointer is null",
            ));
        }
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), buf_native, data.len());
        }

        let max_retries = 100;
        let mut submitted = false;
        for retry in 0..max_retries {
            let result = unsafe { self.component.submit_input(buffer.as_ptr() as *mut AMFData) };

            if retry > 0 && retry % 10 == 0 {
                log::debug!("Decoder::SubmitInput: retry={retry}, result={result:?}");
            }
            if result == AMF_RESULT::AMF_OK || result == AMF_RESULT::AMF_NEED_MORE_INPUT {
                log::debug!("Decoder::SubmitInput: accepted, result={result:?}");
                submitted = true;
                break;
            }
            if result == AMF_RESULT::AMF_INPUT_FULL
                || result == AMF_RESULT::AMF_DECODER_NO_FREE_SURFACES
                || result == AMF_RESULT::AMF_REPEAT
            {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            if result == AMF_RESULT::AMF_EOF {
                submitted = true;
                break;
            }
            return Error::check(result, "AMFComponent::SubmitInput");
        }
        if !submitted {
            return Err(Error::new_custom(
                "Decoder::decode",
                "SubmitInput retry limit exceeded",
            ));
        }

        self.cmd_tx
            .as_ref()
            .unwrap()
            .send(WorkerCommand::Submit(user_data))
            .map_err(|_| Error::new_custom("Decoder::decode", "worker thread terminated"))?;

        Ok(())
    }

    /// デコーダーをフラッシュして残りのフレームを処理する
    pub fn finish(&mut self) -> Result<(), Error> {
        log::debug!("Decoder::finish: calling Drain");
        let result = self.component.drain();
        log::debug!("Decoder::finish: Drain result={result:?}");
        if result != AMF_RESULT::AMF_OK && result != AMF_RESULT::AMF_INPUT_FULL {
            Error::check(result, "AMFComponent::Drain")?;
        }

        let (tx, rx) = mpsc::sync_channel(1);
        self.cmd_tx
            .as_ref()
            .unwrap()
            .send(WorkerCommand::Finish(tx))
            .map_err(|_| Error::new_custom("Decoder::finish", "worker thread terminated"))?;

        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| Error::new_custom("Decoder::finish", "Finish wait timed out"))?;

        Ok(())
    }
}

// 安全性:
// Drop 内でのみ component/context を解放する。
// ワーカースレッドは Drop より先に停止させる。
impl<T: Send + 'static> Drop for Decoder<T> {
    fn drop(&mut self) {
        self.cmd_tx = None;

        if let Some(handle) = self.poll_thread.take() {
            let _ = handle.join();
        }

        let _ = self.component.terminate();
        let _ = self.context.terminate();
    }
}

// ---------------------------------------------------------------------------
// ワーカースレッド
// ---------------------------------------------------------------------------

/// ワーカースレッドのエントリポイント
///
/// メインスレッドから `Submit(T)` コマンドを受け取り、AMFComponent::QueryOutput を
/// ポーリングしてデコード済みフレームを取得し、コールバックを呼び出す。
fn worker<T, F>(component: Component, mut callback: F, cmd_rx: mpsc::Receiver<WorkerCommand<T>>)
where
    T: Send + 'static,
    F: FnMut(DecodedFrame, T) + Send + 'static,
{
    let mut pending: VecDeque<T> = VecDeque::new();
    let mut output_buffer: VecDeque<DecodedFrame> = VecDeque::new();
    let mut finish: Option<mpsc::SyncSender<()>> = None;

    loop {
        if finish.is_some() && pending.is_empty() {
            break;
        }

        let cmd = if pending.is_empty() {
            match cmd_rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            }
        } else {
            match cmd_rx.recv_timeout(Duration::from_millis(1)) {
                Ok(cmd) => cmd,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Err(e) =
                        drain_output(&mut output_buffer, &mut pending, &mut callback, &component)
                    {
                        log::error!("worker: drain_output failed: {e}");
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };

        match cmd {
            WorkerCommand::Submit(t) => {
                pending.push_back(t);
            }
            WorkerCommand::Finish(tx) => {
                finish = Some(tx);
            }
        }
    }

    if let Some(tx) = finish {
        let _ = tx.send(());
    }
}

/// QueryOutput からの出力をバッファに格納し、pending とマッチングしてコールバックを呼び出す
fn drain_output<T, F>(
    output_buffer: &mut VecDeque<DecodedFrame>,
    pending: &mut VecDeque<T>,
    callback: &mut F,
    component: &Component,
) -> Result<(), Error>
where
    T: Send + 'static,
    F: FnMut(DecodedFrame, T),
{
    loop {
        let mut data: *mut AMFData = ptr::null_mut();
        let result = unsafe { component.query_output(&mut data) };
        log::debug!("worker: QueryOutput result={result:?}");
        if result == AMF_RESULT::AMF_REPEAT {
            if !data.is_null() {
                match extract_frame(data as *mut AMFSurface) {
                    Ok(frame) => output_buffer.push_back(frame),
                    Err(e) => log::error!("Failed to extract frame: {e}"),
                }
                continue;
            }
            break;
        }
        if result == AMF_RESULT::AMF_EOF {
            break;
        }
        if result != AMF_RESULT::AMF_OK || data.is_null() {
            break;
        }
        match extract_frame(data as *mut AMFSurface) {
            Ok(frame) => output_buffer.push_back(frame),
            Err(e) => log::error!("Failed to extract frame: {e}"),
        }
    }

    while !output_buffer.is_empty() && !pending.is_empty() {
        let frame = output_buffer.pop_front().unwrap();
        let t = pending.pop_front().unwrap();
        callback(frame, t);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// フレーム抽出 (standalone)
// ---------------------------------------------------------------------------

/// AMFSurface から DecodedFrame を抽出する
///
/// null ポインタやサイズ不整合によるメモリアクセス違反を防ぐため、
/// 各段階で検証を行う。エラー時は `Err` を返す。
fn extract_frame(surface: *mut AMFSurface) -> Result<DecodedFrame, Error> {
    if surface.is_null() {
        return Err(Error::new_custom("extract_frame", "surface is null"));
    }

    let surface = unsafe { Surface::from_raw(surface) }?;

    let result = surface.convert(AMF_MEMORY_TYPE::AMF_MEMORY_HOST);
    if result != AMF_RESULT::AMF_OK {
        return Err(Error::new_custom(
            "extract_frame",
            &format!("Convert failed: {result:?}"),
        ));
    }

    let y_plane = surface.get_plane(AMF_PLANE_TYPE::AMF_PLANE_Y)?;

    let width_raw = y_plane.get_width();
    let height_raw = y_plane.get_height();
    let width = positive_i32_to_usize(width_raw, "extract_frame", "width")?;
    let height = positive_i32_to_usize(height_raw, "extract_frame", "height")?;
    let y_hpitch_raw = y_plane.get_hpitch();
    let y_hpitch = positive_i32_to_usize(y_hpitch_raw, "extract_frame", "y_hpitch")?;
    if y_hpitch < width {
        return Err(Error::new_custom("extract_frame", "y_hpitch < width"));
    }
    let y_native = y_plane.get_native() as *const u8;
    if y_native.is_null() {
        return Err(Error::new_custom("extract_frame", "Y native is null"));
    }

    let y_size = width
        .checked_mul(height)
        .ok_or_else(|| Error::new_custom("extract_frame", "Y plane size overflow"))?;
    let nv12_size = y_size
        .checked_mul(3)
        .and_then(|v| v.checked_div(2))
        .ok_or_else(|| Error::new_custom("extract_frame", "NV12 size overflow"))?;
    let mut frame_data = vec![0u8; nv12_size];

    for row in 0..height {
        unsafe {
            ptr::copy_nonoverlapping(
                y_native.add(row * y_hpitch),
                frame_data.as_mut_ptr().add(row * width),
                width,
            );
        }
    }

    let uv_plane = surface.get_plane(AMF_PLANE_TYPE::AMF_PLANE_UV)?;
    let uv_hpitch_raw = uv_plane.get_hpitch();
    let uv_hpitch = uv_hpitch_raw.max(0) as usize;
    let uv_height_raw = uv_plane.get_height();
    let uv_height = uv_height_raw.max(0) as usize;
    let uv_native = uv_plane.get_native() as *const u8;
    if !uv_native.is_null() && uv_hpitch >= width && uv_height > 0 {
        let max_uv_rows = (nv12_size - y_size) / width;
        let copy_rows = uv_height.min(max_uv_rows);
        for row in 0..copy_rows {
            unsafe {
                ptr::copy_nonoverlapping(
                    uv_native.add(row * uv_hpitch),
                    frame_data.as_mut_ptr().add(y_size + row * width),
                    width,
                );
            }
        }
    }

    Ok(DecodedFrame {
        width,
        height,
        data: frame_data,
    })
}
