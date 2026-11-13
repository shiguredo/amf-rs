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
    /// 含むバッファを渡した場合、2 フレーム目以降の出力でハンドラーにエラーが通知される。
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

        log::debug!("Decoder::decode");

        let max_retries = 100;
        let mut submitted = false;
        let mut result = AMF_RESULT::AMF_FAIL;
        for retry in 0..max_retries {
            result = unsafe { self.component.submit_input(buffer.as_ptr() as *mut AMFData) };

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
            break;
        }
        if !submitted {
            if result == AMF_RESULT::AMF_INPUT_FULL
                || result == AMF_RESULT::AMF_DECODER_NO_FREE_SURFACES
                || result == AMF_RESULT::AMF_REPEAT
            {
                return Err(Error::new_custom(
                    "Decoder::decode",
                    "SubmitInput retry limit exceeded",
                ));
            }
            return Error::check(result, "AMFComponent::SubmitInput");
        }

        // pts に対応する user_data の情報をワーカーに登録する
        self.cmd_tx
            .as_ref()
            .expect("cmd_tx is available until Decoder is dropped")
            .send(WorkerCommand::Submit {
                pts,
                original_pts,
                user_data,
            })
            .map_err(|_| Error::new_custom("Decoder::decode", "worker thread terminated"))?;

        Ok(())
    }

    /// デコーダーをフラッシュして残りのフレームを処理する
    pub fn finish(&mut self) -> Result<(), Error> {
        // 送信側を取り出して保持する。finish 中は `cmd_tx` を None にして
        // decode と再 finish を拒否し、関数を抜けるときにチャネルを閉じる。
        let Some(cmd_tx) = self.cmd_tx.take() else {
            return Err(Error::new_custom(
                "Decoder::finish",
                "decoder is already finished",
            ));
        };

        log::debug!("Decoder::finish: calling Drain");
        let result = self.component.drain();
        log::debug!("Decoder::finish: Drain result={result:?}");
        if result != AMF_RESULT::AMF_OK && result != AMF_RESULT::AMF_INPUT_FULL {
            Error::check(result, "AMFComponent::Drain")?;
        }

        let (tx, rx) = mpsc::sync_channel(1);
        cmd_tx
            .send(WorkerCommand::Finish(tx))
            .map_err(|_| Error::new_custom("Decoder::finish", "worker thread terminated"))?;

        rx.recv()
            .map_err(|_| Error::new_custom("Decoder::finish", "worker thread terminated"))?;

        Ok(())
    }
}

// Safety:
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
    let mut output_buffer: VecDeque<Result<(Surface, amf_pts), Error>> = VecDeque::new();
    let mut finish: Option<mpsc::SyncSender<()>> = None;

    loop {
        if finish.is_some() {
            // Drain 後は、出力がすべて出るか AMF_EOF が返るまでポーリングする。
            // pending と保持中の出力がともに空になった時点で、全入力に対応する
            // 出力が揃っているため、その場合は EOF を待たずに完了してよい。
            let status = drain_output(&mut output_buffer, &component);
            deliver_buffered(&mut output_buffer, &mut pending, &mut handler);
            if status == DrainStatus::Eof {
                flush_buffered_outputs(&mut output_buffer, &mut pending, &mut handler);
                report_unmatched_inputs(&mut pending, &mut handler);
                break;
            }
            if pending.is_empty() && output_buffer.is_empty() {
                break;
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
                    drain_output(&mut output_buffer, &component);
                    deliver_buffered(&mut output_buffer, &mut pending, &mut handler);
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
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), Error>>,
    handler: &mut H,
    finish: &mut Option<mpsc::SyncSender<()>>,
) {
    match cmd {
        WorkerCommand::Submit {
            pts,
            original_pts,
            user_data,
        } => {
            // PTS は `Decoder::decode` が入力ごとに一意に割り当てるため、登録の
            // 重複はない。
            pending.insert(
                pts,
                PendingFrame {
                    original_pts,
                    user_data,
                },
            );
        }
        WorkerCommand::Finish(tx) => {
            *finish = Some(tx);
        }
    }

    // Submit によって対応付けが変わったため、保持している出力を再確認する。
    deliver_buffered(output_buffer, pending, handler);
}

/// QueryOutput のポーリング結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrainStatus {
    /// 現在取得できる出力がない
    Drained,
    /// デコーダーが Drain を完了した
    Eof,
}

/// QueryOutput から出力を取得し、user_data の対応付け待ちのバッファに追加する
fn drain_output(
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), Error>>,
    component: &Component,
) -> DrainStatus {
    let context = component.context();
    loop {
        let mut data: *mut AMFData = ptr::null_mut();
        let result = unsafe { component.query_output(&mut data) };
        log::debug!("worker: QueryOutput result={result:?}");
        // AMF デコーダーは AMF_REPEAT でも有効なデータを返す場合があるため、
        // data が非 null であればフレームとして抽出する。
        if result == AMF_RESULT::AMF_REPEAT {
            if data.is_null() {
                return DrainStatus::Drained;
            }
            output_buffer.push_back(extract_frame(data as *mut AMFSurface, &context));
            continue;
        }
        if result == AMF_RESULT::AMF_EOF {
            return DrainStatus::Eof;
        }
        if result != AMF_RESULT::AMF_OK || data.is_null() {
            return DrainStatus::Drained;
        }
        output_buffer.push_back(extract_frame(data as *mut AMFSurface, &context));
    }
}

/// 保持している出力を先頭から順に、対応する入力フレームが登録されていれば
/// ハンドラーへ通知する
///
/// 先頭の出力に対応する入力フレームがまだ登録されていない場合は、その出力を保持した
/// まま待機する。エラーの出力は対応する入力を必要としないため、その場で通知する。
/// 通知する `Surface` の PTS は、入力時に設定された値へ戻す。
fn deliver_buffered<H: DecodeHandler>(
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), Error>>,
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    handler: &mut H,
) {
    loop {
        match output_buffer.front() {
            None => return,
            Some(Err(_)) => {
                let output = output_buffer
                    .pop_front()
                    .expect("front に対応する出力が存在する");
                let error = output.expect_err("front はエラーの出力である");
                handler.on_decoded(Err(error.into()));
            }
            Some(Ok((_, pts))) => {
                let pts = *pts;
                let Some(frame) = pending.remove(&pts) else {
                    return;
                };
                let output = output_buffer
                    .pop_front()
                    .expect("front に対応する出力が存在する");
                let (surface, _) = output.expect("front は成功した出力である");
                surface.set_pts(frame.original_pts);
                handler.on_decoded(Ok(DecodedFrame {
                    surface,
                    user_data: frame.user_data,
                }));
            }
        }
    }
}

/// Drain 完了時に残っている出力をすべてハンドラーへ通知する
///
/// 対応する入力がない出力はエラーとして通知する。
fn flush_buffered_outputs<H: DecodeHandler>(
    output_buffer: &mut VecDeque<Result<(Surface, amf_pts), Error>>,
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    handler: &mut H,
) {
    while let Some(output) = output_buffer.pop_front() {
        match output {
            Ok((surface, pts)) => match pending.remove(&pts) {
                Some(frame) => {
                    surface.set_pts(frame.original_pts);
                    handler.on_decoded(Ok(DecodedFrame {
                        surface,
                        user_data: frame.user_data,
                    }));
                }
                None => handler.on_decoded(Err(Error::new_custom(
                    "Decoder::worker",
                    &format!("no submitted frame for output PTS {pts}"),
                )
                .into())),
            },
            Err(error) => handler.on_decoded(Err(error.into())),
        }
    }
}

/// 出力が得られなかった入力フレームをエラーとして通知する
fn report_unmatched_inputs<H: DecodeHandler>(
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    handler: &mut H,
) {
    for (pts, _) in pending.drain() {
        handler.on_decoded(Err(Error::new_custom(
            "Decoder::finish",
            &format!("decoder did not output a frame for submitted PTS {pts}"),
        )
        .into()));
    }
}

// ---------------------------------------------------------------------------
// フレーム抽出 (standalone)
// ---------------------------------------------------------------------------

/// AMFSurface を抽出して convert し、PTS と一緒に返す
///
/// `context` には `surface` に対する [Context] を指定する。
/// null ポインタや convert 失敗の場合は `Err` を返す。
fn extract_frame(surface: *mut AMFSurface, context: &Context) -> Result<(Surface, amf_pts), Error> {
    if surface.is_null() {
        return Err(Error::new_custom("extract_frame", "surface is null"));
    }

    // デコーダーの出力 Surface は、デコーダーが作成された Context から確保されている
    let surface = unsafe { Surface::from_raw(surface, context.clone()) }?;

    // Convert が PTS を保持するかに依存せず、メモリ種別を変換する前の Surface から
    // 対応付け用の PTS を取得する。
    let pts = surface.get_pts();

    let result = surface.convert(AMF_MEMORY_TYPE::AMF_MEMORY_HOST);
    if result != AMF_RESULT::AMF_OK {
        return Err(Error::new_custom(
            "extract_frame",
            &format!("Convert failed: {result:?}"),
        ));
    }

    Ok((surface, pts))
}
