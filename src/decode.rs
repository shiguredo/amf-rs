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
use crate::error::{
    AmfError, EnvironmentError, Error, InternalError, InvalidArgumentError, InvalidStateError,
};
use crate::query_output::{DrainStatus, QueryOutputKind, classify_query_output};
use crate::sys::{self, AMF_RESULT, AMFData, AMFSurface, amf_pts};
use crate::types::{FrameFormat, MemoryType, SurfaceFormat};

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

/// デコーダーのコーデック種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderCodec {
    /// H.264/AVC
    H264,
    /// H.265/HEVC
    Hevc,
    /// AV1
    Av1,
}

/// デコーダー設定
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    /// デコードするコーデック
    ///
    /// 出力フォーマットは NV12 に固定される。
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
    /// クロージャからハンドラーを作成する
    ///
    /// `f` は [`DecodeHandler::on_decoded`] と同じ引数で、デコード完了時に
    /// ワーカースレッドから呼ばれる。
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

/// `WaitEof` の結果
enum WaitEofOutcome {
    /// EOF まで回収して通常ループへ戻った
    Drained,
    /// 終端エラーでワーカーが終了した
    DrainFailed,
}

enum WorkerCommand<T> {
    /// 入力フレームの対応付け用 PTS、元の PTS、ユーザーデータを登録する
    Submit {
        pts: amf_pts,
        original_pts: amf_pts,
        user_data: T,
    },
    /// AMF に Drain を呼び、AMF_EOF まで出力を回収して完了を `done` へ送る
    ///
    /// 終端エラーを検出した場合は `done` を送らずにワーカーを終了する。
    WaitEof {
        /// 回収完了を通知するチャネル
        done: mpsc::SyncSender<()>,
    },
    /// ワーカーを停止し、完了を `done` へ送る
    ///
    /// 保持している入出力は通知せずに破棄する。
    Stop {
        /// 停止完了を通知するチャネル
        done: mpsc::SyncSender<()>,
    },
}

// ---------------------------------------------------------------------------
// デコーダー実装
// ---------------------------------------------------------------------------

/// `SubmitInput` をリトライし、入力が受理されるか解像度変化が返るまで待つ
///
/// `AMF_OK`、`AMF_NEED_MORE_INPUT`、`AMF_RESOLUTION_CHANGED`、`AMF_REPEAT` を返す。
/// `AMF_REPEAT` は複数フレームを含むバッファを一部処理したことを示す。
/// その他のエラーは `Err` を返す。
fn submit_input_with_retry(component: &Component, buffer: &Buffer) -> Result<AMF_RESULT, Error> {
    let max_retries = 100;
    let mut last_result = AMF_RESULT::AMF_OK;
    for retry in 0..max_retries {
        let result = unsafe { component.submit_input(buffer.as_ptr() as *mut AMFData) };
        last_result = result;
        if retry > 0 && retry % 10 == 0 {
            tracing::trace!("Decoder::SubmitInput: retry={retry}, result={result:?}");
        }
        if matches!(
            result,
            AMF_RESULT::AMF_OK
                | AMF_RESULT::AMF_NEED_MORE_INPUT
                | AMF_RESULT::AMF_RESOLUTION_CHANGED
                | AMF_RESULT::AMF_REPEAT
        ) {
            return Ok(result);
        }
        if !matches!(
            result,
            AMF_RESULT::AMF_INPUT_FULL | AMF_RESULT::AMF_DECODER_NO_FREE_SURFACES
        ) {
            return Err(AmfError::new(result, "AMFComponent::SubmitInput").into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err(InternalError::new(format!(
        "AMFComponent::SubmitInput did not accept input after {max_retries} retries (last status: {last_result:?})"
    ))
    .into())
}

/// AMF ハードウェアデコーダー
///
/// デコード結果は、[`Decoder::new`] に渡したハンドラーへワーカースレッドから通知される。
///
/// [`Decoder::decode`] / [`Decoder::finish`] がエラーを返した場合、その後は
/// `Drop` 以外の操作を保証しない。`Drop` は安全に呼び出せる。
///
/// # Drop
///
/// [`Decoder::finish`] を呼ばずに drop した場合、ワーカーが保留している入力の
/// `user_data` と出力は通知されずに破棄される。
pub struct Decoder<H: DecodeHandler> {
    component: Component,
    context: Context,
    cmd_tx: Option<mpsc::Sender<WorkerCommand<H::UserData>>>,
    poll_thread: Option<JoinHandle<()>>,
    /// 次に投入する入力フレームへ割り当てる PTS
    next_pts: amf_pts,
    /// AMF が入力を処理していて Drain が必要か
    ///
    /// `SubmitInput` が入力を受理した後、または複数フレームの一部を処理した後に
    /// `true` にする。`WaitEof` の完了後と `Terminate` の後に `false` にする。
    /// `finish` で `Drain` が必要かの判定に使う。
    drain_required: bool,
    /// コンポーネントが初期化済みか
    ///
    /// `Drop` で `Terminate` を呼ぶかの判定に使う。
    initialized: bool,
}

impl<H: DecodeHandler> Decoder<H> {
    /// デコーダーを作成する
    ///
    /// `handler` はデコード完了時にワーカースレッドから呼び出される。
    ///
    /// # Errors
    ///
    /// AMF のデコーダーを初期化できない場合にエラーを返す。GPU と AMF の実装が
    /// 指定したコーデックに対応していない場合も含む。
    pub fn new(context: Context, config: DecoderConfig, handler: H) -> Result<Self, Error> {
        let lib = AmfLibrary::instance();

        let component_id = match config.codec {
            DecoderCodec::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
            DecoderCodec::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
            DecoderCodec::Av1 => sys::str::AMFVideoDecoderHW_AV1,
        };

        let component = lib.create_component(&context, component_id)?;

        set_timestamp_mode(&component)?;

        // デコーダーを初期化する (解像度は 0,0 でストリームから自動検出)
        let result = component.init(SurfaceFormat::from(FrameFormat::Nv12), 0, 0);
        AmfError::check(result, "AMFComponent::Init")?;

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let worker_component = component.clone();
        let poll_thread = std::thread::Builder::new()
            .name("amf-decoder-worker".into())
            .spawn(move || {
                worker(worker_component, handler, cmd_rx);
            });
        let poll_thread = match poll_thread {
            Ok(handle) => handle,
            Err(source) => {
                return Err(
                    EnvironmentError::new("failed to spawn decoder worker thread", source).into(),
                );
            }
        };

        Ok(Self {
            component,
            context,
            cmd_tx: Some(cmd_tx),
            poll_thread: Some(poll_thread),
            next_pts: 1,
            drain_required: false,
            initialized: true,
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
        self.context.alloc_buffer(MemoryType::Host, size)
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
    ///
    /// 解像度が変化するストリームも扱える。解像度の変化を検出した場合も、
    /// 投入したフレームは新しい解像度でデコードされ、他のフレームと同様に
    /// ハンドラーへ通知される。
    ///
    /// # Errors
    ///
    /// `finish` 済みの場合、`buffer` の状態が操作に適していない場合、または
    /// AMF の操作に失敗した場合にエラーを返す。
    pub fn decode(&mut self, buffer: Buffer, user_data: H::UserData) -> Result<(), Error> {
        if self.cmd_tx.is_none() {
            return Err(InvalidStateError::new("decoder is already finished").into());
        }

        // AMF は同じ Context に載ったオブジェクトの受け渡しを前提としているため、
        // 投入前に同じ Context のハンドルであることを確認する。
        if buffer.context() != self.context {
            return Err(InvalidArgumentError::new(
                "buffer does not belong to the decoder's AMF context",
            )
            .into());
        }

        // AMF コンポーネントは投入した Buffer を解放するまで排他アクセスするため、
        // 他のハンドルが残っている状態では投入しない。
        if buffer.is_shared() {
            return Err(InvalidStateError::new(
                "buffer is shared with other handles; drop clones before submitting",
            )
            .into());
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
            .ok_or_else(|| InternalError::new("PTS counter overflowed"))?;
        buffer.set_pts(pts);

        tracing::trace!("Decoder::decode");

        let result = submit_input_with_retry(&self.component, &buffer)?;
        match result {
            AMF_RESULT::AMF_OK | AMF_RESULT::AMF_NEED_MORE_INPUT => {
                // AMF が入力を処理したため、終了時には Drain が必要になる。
                self.drain_required = true;
                tracing::trace!("Decoder::SubmitInput: accepted, result={result:?}");
                // pts に対応する user_data の情報をワーカーに登録する
                self.cmd_tx
                    .as_ref()
                    .expect("cmd_tx is available until Decoder is dropped")
                    .send(WorkerCommand::Submit {
                        pts,
                        original_pts,
                        user_data,
                    })?;
                Ok(())
            }
            AMF_RESULT::AMF_REPEAT => {
                // AMF_REPEAT はバッファの先頭フレームだけを処理したことを示す。
                // 対応付けできないフレームは通知しないが、残りの出力を回収するために
                // finish では Drain が必要になる。
                self.drain_required = true;
                Err(InvalidArgumentError::new(
                    "input buffer contains multiple frames; submit one frame per buffer",
                )
                .into())
            }
            AMF_RESULT::AMF_RESOLUTION_CHANGED => {
                // AMF は解像度の変化を検出すると、このバッファを受理せずに
                // Drain / Terminate / Init によるデコーダーの再作成を要求する。
                // 旧解像度の出力を通知させてからワーカーを待機させ、AMF コンポーネントを
                // 新しい解像度で作り直してバッファを再投入する。
                tracing::debug!("Decoder::decode: resolution changed");
                self.reinitialize_after_resolution_change(buffer, pts, original_pts, user_data)
            }
            // helper の契約上ここには来ない。来た場合はエラーとして扱う。
            _ => Err(AmfError::new(result, "AMFComponent::SubmitInput").into()),
        }
    }

    /// 解像度変化を検出したフレームを新しい解像度のデコーダーへ再投入する
    ///
    /// 成功すると再投入したフレームからワーカーが再開する。失敗した場合は
    /// エラーを返し、このデコーダーは `Drop` 以外の操作を保証しない。
    ///
    /// # Errors
    ///
    /// 解像度変化の処理を完了できない場合にエラーを返す。
    fn reinitialize_after_resolution_change(
        &mut self,
        buffer: Buffer,
        pts: amf_pts,
        original_pts: amf_pts,
        user_data: H::UserData,
    ) -> Result<(), Error> {
        // 旧解像度の出力をすべて通知させ、ワーカーをコマンド待ちに戻す。
        // Drain と AMF_EOF 待ちはワーカーが行う。
        match self.wait_eof_worker()? {
            WaitEofOutcome::Drained => {}
            WaitEofOutcome::DrainFailed => {
                return Err(InternalError::new(
                    "decoder worker terminated before the resolution change completed",
                )
                .into());
            }
        }

        // WaitEof 後はワーカーがコマンド待ちなので、再初期化の間は sender を
        // self から外して保持する。途中で失敗した場合は sender を drop し、
        // ワーカーをチャネル断で終了させる。
        let cmd_tx = self
            .cmd_tx
            .take()
            .ok_or_else(|| InternalError::new("decoder worker thread terminated"))?;

        // AMF_RESOLUTION_CHANGED は Drain / Terminate / Init を要求する
        // (AMF SDK v1.5.2 の include/core/Result.h)。AMF_VIDEO_DECODER_CURRENT_SIZE は
        // 現在の解像度を返す (components/VideoDecoderUVD.h)。AMF の実装によって
        // 変わり得る。
        let property_storage = self.component.property_storage()?;
        let (width, height) =
            property_storage.get_property_size(sys::str::AMF_VIDEO_DECODER_CURRENT_SIZE)?;

        // コンポーネントを作り直す。初期化し直したデコーダーでも入力 PTS を
        // 出力 Surface へ引き継ぐよう、初期化の前に設定を適用する。
        let result = self.component.terminate();
        AmfError::check(result, "AMFComponent::Terminate")?;
        self.initialized = false;
        self.drain_required = false;
        set_timestamp_mode(&self.component)?;
        let result = self
            .component
            .init(SurfaceFormat::from(FrameFormat::Nv12), width, height);
        AmfError::check(result, "AMFComponent::Init")?;
        self.initialized = true;

        // AMF に受理されなかったバッファを新しい解像度のデコーダーへ再投入する。
        let result = submit_input_with_retry(&self.component, &buffer)?;
        match result {
            AMF_RESULT::AMF_OK | AMF_RESULT::AMF_NEED_MORE_INPUT => {
                self.drain_required = true;
            }
            AMF_RESULT::AMF_REPEAT => {
                self.drain_required = true;
                return Err(InvalidArgumentError::new(
                    "input buffer contains multiple frames; submit one frame per buffer",
                )
                .into());
            }
            _ => {
                return Err(AmfError::new(result, "AMFComponent::SubmitInput").into());
            }
        }

        // 再投入したフレームをワーカーへ登録し、通常ループを再開させる。
        cmd_tx.send(WorkerCommand::Submit {
            pts,
            original_pts,
            user_data,
        })?;
        self.cmd_tx = Some(cmd_tx);
        Ok(())
    }

    /// ワーカーに Drain と `AMF_EOF` までの出力回収をさせる
    ///
    /// ワーカーは入力キューが空くまで Drain を再試行し、`AMF_EOF` まで回収して
    /// 完了を通知する。`DrainFailed` の場合はワーカーが終端エラーで終了して
    /// いる。
    fn wait_eof_worker(&mut self) -> Result<WaitEofOutcome, Error> {
        let cmd_tx = match self.cmd_tx.as_ref() {
            Some(cmd_tx) => cmd_tx,
            None => return Err(InternalError::new("decoder worker thread terminated").into()),
        };
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        if cmd_tx
            .send(WorkerCommand::WaitEof { done: done_tx })
            .is_err()
        {
            self.cmd_tx = None;
            self.join_worker()?;
            self.drain_required = false;
            return Err(InternalError::new("decoder worker thread terminated").into());
        }

        match done_rx.recv() {
            Ok(()) => {
                self.drain_required = false;
                Ok(WaitEofOutcome::Drained)
            }
            Err(_) => {
                // ワーカーは終端エラーをハンドラーへ通知して終了したか、panic した。
                // panic は join で検出する。
                self.cmd_tx = None;
                self.join_worker()?;
                self.drain_required = false;
                Ok(WaitEofOutcome::DrainFailed)
            }
        }
    }

    /// ワーカースレッドを join する
    fn join_worker(&mut self) -> Result<(), Error> {
        match self.poll_thread.take() {
            Some(handle) => match handle.join() {
                Ok(()) => Ok(()),
                Err(_) => Err(InternalError::new("decoder worker thread panicked").into()),
            },
            None => Ok(()),
        }
    }

    /// ワーカーを停止する
    fn stop_worker(&mut self) -> Result<(), Error> {
        let cmd_tx = self
            .cmd_tx
            .take()
            .expect("cmd_tx must be set before stopping the worker");
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let worker_replied =
            cmd_tx.send(WorkerCommand::Stop { done: done_tx }).is_ok() && done_rx.recv().is_ok();

        // Stop を送れなかった場合はワーカーの受信側が既に破棄されているため、
        // ワーカーは終了済みか終了処理中である。なので送信結果にかかわらず
        // join で待つ。
        let join_result = match self.poll_thread.take() {
            Some(handle) => handle.join(),
            None => Ok(()),
        };

        match (worker_replied, join_result) {
            (true, Ok(())) => Ok(()),
            // ハンドラーの panic は、panic の内容をエラーに含めずに返す。
            (_, Err(_)) => Err(InternalError::new("decoder worker thread panicked").into()),
            (false, Ok(())) => Err(InternalError::new("decoder worker thread terminated").into()),
        }
    }

    /// 残りのフレームを出力させ、ハンドラーへの通知が完了するまで待つ。
    ///
    /// 処理中に発生したエラーはハンドラーへ通知される。
    ///
    /// # Errors
    ///
    /// `finish` 済みの場合、ワーカーとの通信に失敗した場合、またはワーカーが
    /// 停止している場合にエラーを返す。ハンドラーが panic した場合もワーカーが
    /// 停止したものとして扱う。
    ///
    /// ワーカーが `Drain` または `AMF_EOF` の待機中に終端エラーを検出した場合は、
    /// そのエラーをハンドラーへ通知して `Ok` を返す。
    pub fn finish(&mut self) -> Result<(), Error> {
        if self.cmd_tx.is_none() {
            return Err(InvalidStateError::new("decoder is already finished").into());
        }

        // 入力が受理済みなら、WaitEof でワーカーに Drain と EOF までの回収を
        // させてから Stop で終了する。受理済みの入力がなければ Stop だけを送る。
        // WaitEof 中の終端エラーはハンドラーへ通知済みなので Ok を返す。
        // ワーカーの panic と停止は Err として返す。
        if self.drain_required {
            match self.wait_eof_worker()? {
                WaitEofOutcome::Drained => {}
                WaitEofOutcome::DrainFailed => return Ok(()),
            }
        }
        self.stop_worker()?;
        Ok(())
    }
}

// SAFETY:
// Drop 内でのみ component を解放する。ワーカースレッドは Drop より先に停止させる。
impl<H: DecodeHandler> Drop for Decoder<H> {
    fn drop(&mut self) {
        self.cmd_tx = None;

        if let Some(handle) = self.poll_thread.take() {
            // Drop はエラーを返せないため、ハンドラーの panic はログに記録する。
            if handle.join().is_err() {
                tracing::error!("Decoder::drop: decoder worker thread panicked");
            }
        }

        // 再初期化の Terminate 後に Init が失敗している場合は、既に
        // Terminate 済みなので再度呼ばない。
        if self.initialized {
            // 戻り値を返せない後始末であるため、失敗は記録だけ行う。
            let result = self.component.terminate();
            if result != AMF_RESULT::AMF_OK {
                tracing::error!("Decoder::drop: AMFComponent::Terminate failed: {result:?}");
            }
        }
    }
}

/// 入力 Buffer の PTS を出力 Surface へ引き継ぐモードを設定する
///
/// # Errors
///
/// プロパティを設定できない場合にエラーを返す。
fn set_timestamp_mode(component: &Component) -> Result<(), Error> {
    // AMF Video Decode API の AMF_TIMESTAMP_MODE の説明は資料によって異なるが、
    // VideoDecoderUVD.h は既定の AMF_TS_PRESENTATION が入力のタイムスタンプを
    // 保持すると説明している。実際の H.264/HEVC デコーダーでも
    // AMF_TS_PRESENTATION は入力 PTS を出力 Surface へ引き継ぐ。
    //
    // 参考として、AMF_TS_SORT は入力タイムスタンプを引き継いだあと出力を
    // 昇順にソートし、AMF_TS_DECODE は引き継ぐがソートしない
    // (AMF Video Decode API)。このデコーダーは PTS の一致で user_data を
    // 引き当てるため、入力 PTS の保持をヘッダーと実機で確認できる
    // AMF_TS_PRESENTATION を明示的に設定する。
    let property_storage = component.property_storage()?;
    property_storage.set_property_int64(
        sys::str::AMF_TIMESTAMP_MODE,
        i64::from(sys::AMF_TIMESTAMP_MODE_ENUM_AMF_TS_PRESENTATION),
    )
}

// ---------------------------------------------------------------------------
// ワーカースレッド
// ---------------------------------------------------------------------------

/// ワーカースレッドのエントリポイント
///
/// 呼び出し元スレッドから `Submit` / `WaitEof` / `Stop` コマンドを受け取り、
/// AMFComponent::QueryOutput をポーリングしてデコード済みフレームを取得し、
/// ハンドラーを呼び出す。
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
    let mut wait_eof: Option<mpsc::SyncSender<()>> = None;

    loop {
        if let Some(done) = wait_eof.take() {
            // `WaitEof` は AMF の Drain を受理させ、AMF_EOF まで出力を回収する。
            // Drain が AMF_INPUT_FULL を返す間は QueryOutput を回して入力
            // キューを空けながら再試行する。
            loop {
                let result = component.drain();
                tracing::debug!("worker: Drain result={result:?}");
                if result == AMF_RESULT::AMF_OK {
                    break;
                }
                if result != AMF_RESULT::AMF_INPUT_FULL {
                    notify_terminal_error(
                        AmfError::new(result, "AMFComponent::Drain").into(),
                        &mut output_buffer,
                        &mut pending,
                        &mut handler,
                    );
                    return;
                }
                match drain_output(&mut output_buffer, &component) {
                    Ok(status) => {
                        deliver_buffered(&mut output_buffer, &mut pending, &mut handler);
                        if status == DrainStatus::Eof {
                            break;
                        }
                    }
                    Err(error) => {
                        notify_terminal_error(
                            error,
                            &mut output_buffer,
                            &mut pending,
                            &mut handler,
                        );
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }

            // Drain 受理後、AMF_EOF まで出力を回収する。
            loop {
                match drain_output(&mut output_buffer, &component) {
                    Ok(status) => {
                        deliver_buffered(&mut output_buffer, &mut pending, &mut handler);
                        if status == DrainStatus::Eof {
                            flush_buffered_outputs(&mut output_buffer, &mut pending, &mut handler);
                            report_unmatched_inputs(&mut pending, &mut handler);
                            // 呼び出し側へ回収完了を通知する。送信に失敗した場合は
                            // 呼び出し側が待っていない。
                            let _ = done.send(());
                            break;
                        }
                    }
                    Err(error) => {
                        notify_terminal_error(
                            error,
                            &mut output_buffer,
                            &mut pending,
                            &mut handler,
                        );
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
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
                deliver_buffered(&mut output_buffer, &mut pending, &mut handler);
            }
            WorkerCommand::WaitEof { done } => {
                wait_eof = Some(done);
            }
            WorkerCommand::Stop { done } => {
                let _ = done.send(());
                break;
            }
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
                return Err(AmfError::new(result, "AMFComponent::QueryOutput").into());
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
                None => handler.on_decoded(Err(Error::Internal(InternalError::new(format!(
                    "the decoder produced output that matches no submitted input (internal PTS {pts})"
                )))
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

/// 出力が得られなかった入力を `Err` として通知する。
///
/// 通知順は入力順とする。
fn report_unmatched_inputs<H: DecodeHandler>(
    pending: &mut HashMap<amf_pts, PendingFrame<H::UserData>>,
    handler: &mut H,
) {
    let mut frames: Vec<_> = pending.drain().collect();
    frames.sort_by_key(|(pts, _)| *pts);
    for (_, frame) in frames {
        handler.on_decoded(Err(Error::Internal(InternalError::new(format!(
            "the decoder did not produce output for the submitted input (PTS {})",
            frame.original_pts
        )))
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
    report_unmatched_inputs(pending, handler);
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
            error: InternalError::new("output surface is null").into(),
        });
    }

    // デコーダーの出力 Surface は、デコーダーが作成された Context から確保されている
    let surface = unsafe { Surface::from_raw(surface, context.clone()) }
        .map_err(|error| FrameError { pts: None, error })?;

    // Convert が PTS を保持するかに依存せず、メモリ種別を変換する前の Surface から
    // 対応付け用の PTS を取得する。
    let pts = surface.get_pts();

    surface
        .convert(MemoryType::Host)
        .map_err(|error| FrameError {
            pts: Some(pts),
            error,
        })?;

    Ok((surface, pts))
}
