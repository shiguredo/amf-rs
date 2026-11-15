//! AMF ハードウェアデコーダー
//!
//! AMD GPU を使ったハードウェアビデオデコードを提供する。
//! H.264/AVC、H.265/HEVC、AV1 コーデックに対応する。

use std::collections::{HashMap, VecDeque};
use std::ptr;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::AmfLibrary;
use crate::amf::{Buffer, Component, Context, Surface};
use crate::error::Error;
use crate::query_output::{DrainStatus, QueryOutputKind, classify_query_output};
use crate::sys::{
    self, AMF_MEMORY_TYPE, AMF_RESULT, AMF_SURFACE_FORMAT, AMFData, AMFSurface, amf_pts,
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

    /// ユーザーデータ
    pub fn user_data(&self) -> &T {
        &self.user_data
    }

    /// Surface とユーザーデータの所有権を取得する
    pub fn into_parts(self) -> (Surface, T) {
        (self.surface, self.user_data)
    }
}

// ---------------------------------------------------------------------------
// ハンドラートレイト
// ---------------------------------------------------------------------------

/// デコード結果を通知するためのハンドラー
///
/// デコード処理が完了するたびに [`DecodeHandler::on_decoded`] が呼ばれる。
pub trait DecodeHandler: Send + 'static {
    /// ユーザーデータ型
    type UserData: Send + 'static;
    /// エラー型
    type Error: From<crate::Error> + Send + 'static;
    /// デコード完了時に呼ばれる
    fn on_decoded(&mut self, result: Result<DecodedFrame<Self::UserData>, Self::Error>);
}

/// `FnMut` クロージャを [`DecodeHandler`] にするラッパー
pub struct FnDecodeHandler<T, E = crate::Error> {
    f: Box<dyn FnMut(Result<DecodedFrame<T>, E>) + Send + 'static>,
}

impl<T, E> FnDecodeHandler<T, E> {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(Result<DecodedFrame<T>, E>) + Send + 'static,
    {
        Self { f: Box::new(f) }
    }
}

impl<T, E> DecodeHandler for FnDecodeHandler<T, E>
where
    T: Send + 'static,
    E: From<crate::Error> + Send + 'static,
{
    type UserData = T;
    type Error = E;
    fn on_decoded(&mut self, result: Result<DecodedFrame<T>, E>) {
        (self.f)(result);
    }
}

// ---------------------------------------------------------------------------
// ワーカースレッド用コマンド
// ---------------------------------------------------------------------------

/// 出力との対応付けが完了するまで保持する入力フレームの情報
struct PendingFrame<T> {
    /// 呼び出し元が入力 Buffer に設定した PTS
    original_pts: amf_pts,
    /// デコード完了時にハンドラーへ渡すユーザーデータ
    user_data: T,
}

/// 出力 Surface の抽出エラー
struct FrameError {
    /// このエラーを通知する対象の入力 PTS。対応付けできない場合は None
    pts: Option<amf_pts>,
    /// ハンドラーへ通知するエラー
    error: Error,
}

enum WorkerCommand<T> {
    /// 入力フレームの対応付け用 PTS、元の PTS、ユーザーデータを登録する
    Submit {
        pts: amf_pts,
        original_pts: amf_pts,
        user_data: T,
    },
    Finish(mpsc::SyncSender<()>),
}

// ---------------------------------------------------------------------------
// デコーダー実装
// ---------------------------------------------------------------------------

/// AMF ハードウェアデコーダー
pub struct Decoder<H: DecodeHandler> {
    component: Component,
    context: Context,
    cmd_tx: Option<mpsc::Sender<WorkerCommand<H::UserData>>>,
    poll_thread: Option<JoinHandle<()>>,
    /// 次に投入する入力フレームへ割り当てる PTS
    next_pts: amf_pts,
}

impl<H: DecodeHandler> Decoder<H> {
    /// デコーダーを作成する
    ///
    /// `handler` はデコード完了時にワーカースレッドから呼び出される。
    pub fn new(context: Context, config: DecoderConfig, handler: H) -> Result<Self, Error> {
        let lib = AmfLibrary::instance();

        let component_id = match config.codec {
            DecoderCodec::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
            DecoderCodec::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
            DecoderCodec::Av1 => sys::str::AMFVideoDecoderHW_AV1,
        };

        let component = lib.create_component(&context, component_id)?;

        // 入力 Buffer の PTS を出力 Surface へそのまま引き継ぐモードにする。
        // AMF Video Decode API の AMF_TIMESTAMP_MODE の説明は資料によって異なるが、
        // VideoDecoderUVD.h は既定の AMF_TS_PRESENTATION が入力のタイムスタンプを
        // 保持すると説明している。実際の H.264/HEVC デコーダーでも
        // AMF_TS_PRESENTATION は入力 PTS を出力 Surface へ引き継ぐ。一方
        // AMF_TS_DECODE と AMF_TS_SORT は出力順にタイムスタンプを振り直すため、
        // 入力と出力の対応付けには使えない。このデコーダーは PTS の一致で
        // user_data を引き当てるため、AMF_TS_PRESENTATION を明示的に設定する。
        let property_storage = component.property_storage()?;
        property_storage.set_property_int64(
            sys::str::AMF_TIMESTAMP_MODE,
            i64::from(sys::AMF_TIMESTAMP_MODE_ENUM_AMF_TS_PRESENTATION),
        )?;

        // デコーダーを初期化する (解像度は 0,0 でストリームから自動検出)
        let result = component.init(AMF_SURFACE_FORMAT::AMF_SURFACE_NV12, 0, 0);
        Error::check(result, "AMFComponent::Init")?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand<H::UserData>>();
        let worker_component = component.clone();
        let poll_thread = std::thread::Builder::new()
            .name("amf-decoder-worker".into())
            .spawn(move || {
                worker(worker_component, handler, cmd_rx);
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
            next_pts: 1,
        })
    }

    /// このデコーダーが使用する `Context` を返す
    pub fn context(&self) -> Context {
        self.context.clone()
    }

    /// デコード用のバッファを確保する
    ///
    /// 確保されたバッファは呼び出し元がビットストリームデータを書き込んでから
    /// [`Decoder::decode`] に渡す。
    pub fn alloc_buffer(&self, size: usize) -> Result<Buffer, Error> {
        self.context
            .alloc_buffer(AMF_MEMORY_TYPE::AMF_MEMORY_HOST, size)
    }

    /// ビットストリームをデコードする
    ///
    /// `user_data` はデコード完了時にハンドラーへ渡される。
    ///
    /// `buffer` には 1 フレーム分のビットストリームを格納すること。複数フレームを
    /// 含むバッファを渡した場合の挙動は実装依存となる。
    ///
    /// `buffer` はこのデコーダーと同じ `Context` から確保したものでなければならない。
    /// 別の Context から確保した Buffer を渡した場合はエラーを返す。`buffer` が他の
    /// ハンドルと共有されている場合もエラーを返す。
    pub fn decode(&mut self, buffer: Buffer, user_data: H::UserData) -> Result<(), Error> {
        if self.cmd_tx.is_none() {
            return Err(Error::new_custom(
                "Decoder::decode",
                "decoder is already finished",
            ));
        }

        // AMF は同じ Context に載ったオブジェクトの受け渡しを前提としているため、
        // 投入前に同じ Context のハンドルであることを確認する。
        if buffer.context() != self.context {
            return Err(Error::new_custom(
                "Decoder::decode",
                "buffer does not belong to the decoder's AMF context",
            ));
        }

        // AMF コンポーネントは投入した Buffer を解放するまで排他アクセスするため、
        // 他のハンドルが残っている状態では投入しない。
        if buffer.is_shared() {
            return Err(Error::new_custom(
                "Decoder::decode",
                "buffer is shared with other handles; drop clones before submitting",
            ));
        }

        // 呼び出し元が入力 Buffer に設定した PTS を保存する。AMF のデコーダーは
        // 入力 PTS を出力 Surface へ引き継ぐため、対応付け用の ID で上書きする前に
        // 元の値を取得しておき、通知前に出力 Surface へ戻す。
        let original_pts = buffer.get_pts();

        // 入力と出力の対応付けに使う PTS を割り当てる。提示時刻ではなく一意な ID と
        // して扱うため、1 から単調増加させる。
        let pts = self.next_pts;
        self.next_pts = self
            .next_pts
            .checked_add(1)
            .ok_or_else(|| Error::new_custom("Decoder::decode", "PTS counter overflowed"))?;
        buffer.set_pts(pts);

        tracing::trace!("Decoder::decode");

        let max_retries = 100;
        for retry in 0..max_retries {
            let result = unsafe { self.component.submit_input(buffer.as_ptr() as *mut AMFData) };

            if retry > 0 && retry % 10 == 0 {
                tracing::trace!("Decoder::SubmitInput: retry={retry}, result={result:?}");
            }
            if matches!(result, AMF_RESULT::AMF_OK | AMF_RESULT::AMF_NEED_MORE_INPUT) {
                tracing::trace!("Decoder::SubmitInput: accepted, result={result:?}");
                // pts に対応する user_data の情報をワーカーに登録する
                self.cmd_tx
                    .as_ref()
                    .expect("cmd_tx is available until Decoder is dropped")
                    .send(WorkerCommand::Submit {
                        pts,
                        original_pts,
                        user_data,
                    })
                    .map_err(|_| {
                        Error::new_custom("Decoder::decode", "worker thread terminated")
                    })?;
                return Ok(());
            }
            if result == AMF_RESULT::AMF_REPEAT {
                // AMF_REPEAT はバッファに入っているフレーム数が多すぎて受理できなかったことを示す。
                // この API は 1 バッファ 1 フレームを前提とするため、エラーにする。
                return Err(Error::new_custom(
                    "Decoder::decode",
                    "input buffer contains multiple frames; submit one frame per buffer",
                ));
            }
            if !matches!(
                result,
                AMF_RESULT::AMF_INPUT_FULL | AMF_RESULT::AMF_DECODER_NO_FREE_SURFACES
            ) {
                return Error::check(result, "AMFComponent::SubmitInput");
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        Err(Error::new_custom(
            "Decoder::decode",
            "SubmitInput retry limit exceeded",
        ))
    }

    /// 残りのフレームを出力させ、ハンドラーへの通知が完了するまで待つ。
    ///
    /// 処理中に発生したエラーはハンドラーへ通知される。ワーカーが既に終了している
    /// 場合や Drain に失敗した場合は `Err` を返す。
    ///
    /// `finish` が成功した後に再度 `finish` を呼ぶと `Err` を返す。
    pub fn finish(&mut self) -> Result<(), Error> {
        if self.cmd_tx.is_none() {
            return Err(Error::new_custom(
                "Decoder::finish",
                "decoder is already finished",
            ));
        }

        tracing::debug!("Decoder::finish: calling Drain");
        let result = self.component.drain();
        tracing::debug!("Decoder::finish: Drain result={result:?}");
        if result != AMF_RESULT::AMF_OK && result != AMF_RESULT::AMF_INPUT_FULL {
            Error::check(result, "AMFComponent::Drain")?;
        }

        // Drain が成功してから `cmd_tx` を取り出す。finish 中は `cmd_tx` を None に
        // して decode と再 finish を拒否し、関数を抜けるときにチャネルを閉じる。
        let cmd_tx = self
            .cmd_tx
            .take()
            .expect("cmd_tx must be set before finish");
        let (tx, rx) = mpsc::sync_channel(1);
        cmd_tx
            .send(WorkerCommand::Finish(tx))
            .map_err(|_| Error::new_custom("Decoder::finish", "worker thread terminated"))?;

        rx.recv()
            .map_err(|_| Error::new_custom("Decoder::finish", "worker thread terminated"))?;

        Ok(())
    }
}

// SAFETY:
// Drop 内でのみ component を解放する。ワーカースレッドは Drop より先に停止させる。
impl<H: DecodeHandler> Drop for Decoder<H> {
    fn drop(&mut self) {
        self.cmd_tx = None;

        if let Some(handle) = self.poll_thread.take() {
            let _ = handle.join();
        }

        let _ = self.component.terminate();
    }
}

// ---------------------------------------------------------------------------
// ワーカースレッド
// ---------------------------------------------------------------------------

/// ワーカースレッドのエントリポイント
///
/// メインスレッドから `Submit` コマンドを受け取り、AMFComponent::QueryOutput を
/// ポーリングしてデコード済みフレームを取得し、ハンドラーを呼び出す。
fn worker<H: DecodeHandler>(
    component: Component,
    mut handler: H,
    cmd_rx: mpsc::Receiver<WorkerCommand<H::UserData>>,
) {
    // 対応付け用 PTS から入力フレームの情報を引くための表。PTS は
    // `Decoder::decode` が入力ごとに一意に割り当てるため、値の重複はない。
    let mut pending: HashMap<amf_pts, PendingFrame<H::UserData>> = HashMap::new();
    // Submit コマンドの処理が出力の取得より遅れた場合に備え、対応する user_data が
    // 登録されるまで出力を保持する。
    let mut output_buffer: VecDeque<Result<(Surface, amf_pts), FrameError>> = VecDeque::new();
    let mut finish: Option<mpsc::SyncSender<()>> = None;

    loop {
        if finish.is_some() {
            // Drain 後は、出力がすべて出るか AMF_EOF が返るまでポーリングする。
            // pending と保持中の出力がともに空になった時点で、このワーカーが保持する
            // 入出力はなくなっているため、その場合は EOF を待たずに完了してよい。
            match drain_output(&mut output_buffer, &component) {
                Ok(status) => {
                    deliver_buffered(&mut output_buffer, &mut pending, &mut handler);
                    if status == DrainStatus::Eof {
                        flush_buffered_outputs(&mut output_buffer, &mut pending, &mut handler);
                        report_unmatched_inputs(&mut pending, &mut handler, "Decoder::finish");
                        break;
                    }
                    if pending.is_empty() && output_buffer.is_empty() {
                        break;
                    }
                }
                Err(error) => {
                    // 終端エラーでは AMF_EOF を待たず、取得済みの出力と残りの入力を
                    // 通知してワーカーを終了する。
                    notify_terminal_error(error, &mut output_buffer, &mut pending, &mut handler);
                    break;
                }
            }

            std::thread::sleep(Duration::from_millis(1));
            continue;
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
                    match drain_output(&mut output_buffer, &component) {
                        Ok(_) => {
                            deliver_buffered(&mut output_buffer, &mut pending, &mut handler);
                        }
                        Err(error) => {
                            // 通常のデコード中でも終端エラーは起き得るため、同じように
                            // 取得済みの出力と残りの入力を通知して終了する。
                            notify_terminal_error(
                                error,
                                &mut output_buffer,
                                &mut pending,
                                &mut handler,
                            );
                            break;
                        }
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };

        handle_command(
            cmd,
            &mut pending,
            &mut output_buffer,
            &mut handler,
            &mut finish,
        );
    }

    if let Some(tx) = finish {
        let _ = tx.send(());
    }
}

/// ワーカーコマンドを処理する
fn handle_command<H: DecodeHandler>(
    cmd: WorkerCommand<H::UserData>,
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), FrameError>>,
    handler: &mut H,
    finish: &mut Option<mpsc::SyncSender<()>>,
) {
    match cmd {
        WorkerCommand::Submit {
            pts,
            original_pts,
            user_data,
        } => {
            pending.insert(
                pts,
                PendingFrame {
                    original_pts,
                    user_data,
                },
            );
            // Submit によって対応付けが変わったため、保持している出力を再確認する。
            deliver_buffered(output_buffer, pending, handler);
        }
        WorkerCommand::Finish(tx) => {
            *finish = Some(tx);
        }
    }
}

/// `QueryOutput` から取得できた出力を `output_buffer` に追加する。
///
/// 取得できる出力がなくなった場合は [`DrainStatus::NotReady`]、ドレインが完了した
/// 場合は [`DrainStatus::Eof`] を返す。終端エラーの場合は `Err` を返す。
fn drain_output(
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), FrameError>>,
    component: &Component,
) -> Result<DrainStatus, Error> {
    let context = component.context();
    loop {
        let mut data: *mut AMFData = ptr::null_mut();
        let result = unsafe { component.query_output(&mut data) };
        tracing::trace!("worker: QueryOutput result={result:?}");
        let kind = classify_query_output(result, !data.is_null());
        // QueryOutput が返した参照は呼び出し元が所有する。
        if kind != QueryOutputKind::Output && !data.is_null() {
            let vtbl = unsafe { &*(*data).pVtbl };
            if let Some(release) = vtbl.Release {
                unsafe { release(data) };
            }
        }
        match kind {
            QueryOutputKind::Output => {
                output_buffer.push_back(extract_frame(data as *mut AMFSurface, &context));
            }
            QueryOutputKind::NoOutput => {
                return Ok(DrainStatus::NotReady);
            }
            QueryOutputKind::Eof => {
                return Ok(DrainStatus::Eof);
            }
            QueryOutputKind::TerminalError => {
                return Err(Error::from_amf(result, "AMFComponent::QueryOutput"));
            }
        }
    }
}

/// 入力時の PTS に戻した `Surface` と、対応するユーザーデータを組み合わせて返す。
fn into_decoded_frame<H: DecodeHandler>(
    surface: Surface,
    frame: PendingFrame<H::UserData>,
) -> DecodedFrame<H::UserData> {
    surface.set_pts(frame.original_pts);
    DecodedFrame {
        surface,
        user_data: frame.user_data,
    }
}

/// 対応する入力がある出力を、出力順にハンドラーへ通知する。
///
/// 対応する入力がまだない出力は通知せずに保持する。出力の抽出エラーは `Err` として
/// 通知する。通知する `Surface` の PTS は、入力時に設定された値へ戻す。
fn deliver_buffered<H: DecodeHandler>(
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), FrameError>>,
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    handler: &mut H,
) {
    loop {
        match output_buffer.front() {
            None => return,
            Some(Err(_)) => {
                let output = output_buffer.pop_front().expect("front output must exist");
                let frame_error = match output {
                    Err(error) => error,
                    Ok(_) => unreachable!("front output must be an error"),
                };
                // エラーを通知した入力は、未対応入力として再通知しない。
                if let Some(pts) = frame_error.pts {
                    pending.remove(&pts);
                }
                handler.on_decoded(Err(frame_error.error.into()));
            }
            Some(Ok((_, pts))) => {
                let pts = *pts;
                let Some(frame) = pending.remove(&pts) else {
                    return;
                };
                let output = output_buffer.pop_front().expect("front output must exist");
                let (surface, _) = match output {
                    Ok(output) => output,
                    Err(_) => unreachable!("front output must be a success"),
                };
                handler.on_decoded(Ok(into_decoded_frame::<H>(surface, frame)));
            }
        }
    }
}

/// 保持している出力をすべてハンドラーへ通知する。
///
/// 対応する入力がある出力は `Ok`、ない出力は `Err` として通知する。抽出エラーも
/// `Err` として通知する。ドレイン完了時と終端エラー時に呼ぶ。
fn flush_buffered_outputs<H: DecodeHandler>(
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), FrameError>>,
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    handler: &mut H,
) {
    while let Some(output) = output_buffer.pop_front() {
        match output {
            Ok((surface, pts)) => match pending.remove(&pts) {
                Some(frame) => {
                    handler.on_decoded(Ok(into_decoded_frame::<H>(surface, frame)));
                }
                None => handler.on_decoded(Err(Error::new_custom(
                    "Decoder::worker",
                    &format!("no submitted frame for output with internal PTS {pts}"),
                )
                .into())),
            },
            Err(frame_error) => {
                // エラーを通知した入力は、未対応入力として再通知しない。
                if let Some(pts) = frame_error.pts {
                    pending.remove(&pts);
                }
                handler.on_decoded(Err(frame_error.error.into()));
            }
        }
    }
}

/// 出力が得られなかった入力を、`function` を発生元とする `Err` として通知する。
///
/// 通知順は入力順とする。
fn report_unmatched_inputs<H: DecodeHandler>(
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    handler: &mut H,
    function: &'static str,
) {
    let mut frames: Vec<_> = pending.drain().collect();
    frames.sort_by_key(|(pts, _)| *pts);
    for (_, frame) in frames {
        handler.on_decoded(Err(Error::new_custom(
            function,
            &format!(
                "decoder did not output a frame for submitted PTS {}",
                frame.original_pts
            ),
        )
        .into()));
    }
}

/// 終端エラー発生時に、保持している出力、原因のエラー、出力が得られなかった入力の
/// 順にハンドラーへ通知する。
fn notify_terminal_error<H: DecodeHandler>(
    error: Error,
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), FrameError>>,
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    handler: &mut H,
) {
    flush_buffered_outputs(output_buffer, pending, handler);
    handler.on_decoded(Err(error.into()));
    report_unmatched_inputs(pending, handler, "Decoder::worker");
}

// ---------------------------------------------------------------------------
// フレーム抽出 (standalone)
// ---------------------------------------------------------------------------

/// デコード済みの `Surface` と、対応付けに使った PTS を返す。
///
/// `context` には `surface` に対する [Context] を指定する。
/// `Surface` を返せない場合は `Err` を返す。
fn extract_frame(
    surface: *mut AMFSurface,
    context: &Context,
) -> Result<(Surface, amf_pts), FrameError> {
    if surface.is_null() {
        return Err(FrameError {
            pts: None,
            error: Error::new_custom("extract_frame", "surface is null"),
        });
    }

    // デコーダーの出力 Surface は、デコーダーが作成された Context から確保されている
    let surface = unsafe { Surface::from_raw(surface, context.clone()) }
        .map_err(|error| FrameError { pts: None, error })?;

    // Convert が PTS を保持するかに依存せず、メモリ種別を変換する前の Surface から
    // 対応付け用の PTS を取得する。
    let pts = surface.get_pts();

    surface
        .convert(AMF_MEMORY_TYPE::AMF_MEMORY_HOST)
        .map_err(|error| FrameError {
            pts: Some(pts),
            error,
        })?;

    Ok((surface, pts))
}
