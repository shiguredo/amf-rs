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
use crate::amf::{Buffer, Component, Context, Surface};
use crate::error::Error;
use crate::sys::{
    self, AMF_MEMORY_TYPE, AMF_RESULT, AMF_SURFACE_FORMAT, AMFData, AMFSurface,
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

/// デコード済みフレーム
#[derive(Debug)]
pub struct DecodedFrame<T> {
    surface: Surface,
    user_data: T,
}

impl<T> DecodedFrame<T> {
    /// デコード後の Surface (convert 済み)
    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Surface の所有権を取得する
    pub fn into_surface(self) -> Surface {
        self.surface
    }

    /// ユーザーデータ
    pub fn user_data(&self) -> &T {
        &self.user_data
    }

    /// ユーザーデータの所有権を取得する
    pub fn into_user_data(self) -> T {
        self.user_data
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
    /// 引数は `decode()` に渡されたユーザーデータを含むデコード済みフレーム。
    pub fn new(
        config: DecoderConfig,
        callback: impl FnMut(DecodedFrame<T>) + Send + 'static,
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

    /// デコード用のバッファを確保する
    ///
    /// 確保されたバッファは呼び出し元がビットストリームデータを書き込んでから
    /// [`decode()`] に渡す。
    pub fn alloc_buffer(&self, size: usize) -> Result<Buffer, Error> {
        self.context
            .alloc_buffer(AMF_MEMORY_TYPE::AMF_MEMORY_HOST, size)
    }

    /// ビットストリームをデコードする
    ///
    /// `user_data` はデコード完了時にコールバックへ渡される。
    pub fn decode(&mut self, buffer: Buffer, user_data: T) -> Result<(), Error> {
        log::debug!("Decoder::decode");

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
    F: FnMut(DecodedFrame<T>) + Send + 'static,
{
    let mut pending: VecDeque<T> = VecDeque::new();
    let mut output_buffer: VecDeque<Surface> = VecDeque::new();
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
    output_buffer: &mut VecDeque<Surface>,
    pending: &mut VecDeque<T>,
    callback: &mut F,
    component: &Component,
) -> Result<(), Error>
where
    T: Send + 'static,
    F: FnMut(DecodedFrame<T>),
{
    loop {
        let mut data: *mut AMFData = ptr::null_mut();
        let result = unsafe { component.query_output(&mut data) };
        log::debug!("worker: QueryOutput result={result:?}");
        if result == AMF_RESULT::AMF_REPEAT {
            if !data.is_null() {
                match extract_frame(data as *mut AMFSurface) {
                    Ok(surface) => output_buffer.push_back(surface),
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
            Ok(surface) => output_buffer.push_back(surface),
            Err(e) => log::error!("Failed to extract frame: {e}"),
        }
    }

    while !output_buffer.is_empty() && !pending.is_empty() {
        let surface = output_buffer.pop_front().unwrap();
        let user_data = pending.pop_front().unwrap();
        callback(DecodedFrame { surface, user_data });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// フレーム抽出 (standalone)
// ---------------------------------------------------------------------------

/// AMFSurface を抽出して convert する
///
/// null ポインタや convert 失敗の場合は `Err` を返す。
fn extract_frame(surface: *mut AMFSurface) -> Result<Surface, Error> {
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

    Ok(surface)
}
