//! エンコーダーとデコーダーで共通のワーカースレッド
//!
//! `AMFComponent::SubmitInput` のリトライ、`QueryOutput` のポーリング、
//! frame ID による入出力の対応付け、ハンドラーへの通知、終了処理を提供する。

use std::collections::{HashMap, VecDeque};
use std::ptr;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::amf::{Component, Data};
use crate::error::{AmfError, EnvironmentError, Error, InternalError, InvalidStateError};
use crate::query_output::{DrainStatus, QueryOutputKind, classify_query_output};
use crate::sys::{AMF_RESULT, AMFData};

/// 入出力の対応付けに使う frame ID を保持するプロパティ名
///
/// ライブラリがこの名前のプロパティを入力フレームに設定し、同じフレームの
/// 出力から読み取る。利用者はこの名前を予約名として扱い、自分で設定しないこと。
const FRAME_ID_PROPERTY: &str = "ShiguredoAmfFrameId";

/// `AMFComponent::SubmitInput` のリトライ回数の上限
const MAX_SUBMIT_RETRIES: usize = 100;

/// `SubmitInput` のリトライ間隔
const SUBMIT_RETRY_INTERVAL: Duration = Duration::from_millis(1);

/// ワーカーがコマンドを待つ間隔
///
/// 保留中の入力や出力がある間は、この間隔で `QueryOutput` を呼んで出力を回収する。
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// 入力と出力の対応付けに使う frame ID を発行する
struct FrameIdGenerator {
    /// 次に発行する値
    next: i64,
}

impl FrameIdGenerator {
    /// 1 から始まる frame ID の発行器を作成する
    fn new() -> Self {
        Self { next: 1 }
    }

    /// 新しい frame ID を発行する
    ///
    /// # Errors
    ///
    /// 発行できる値の上限に達した場合にエラーを返す。
    fn next(&mut self) -> Result<i64, Error> {
        let frame_id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| InternalError::new("frame ID counter overflowed"))?;
        Ok(frame_id)
    }
}

/// `AMFComponent::SubmitInput` をリトライするステータス
///
/// `AMF_DECODER_NO_FREE_SURFACES` はデコーダー専用のステータスだが、出力を
/// 回収することで解消する一時的なリソース枯渇を表すため、エンコーダーと共通の
/// リトライ対象として扱う。エンコーダーはこのステータスを返さない。
const SUBMIT_RETRY_STATUSES: &[AMF_RESULT] = &[
    AMF_RESULT::AMF_INPUT_FULL,
    AMF_RESULT::AMF_DECODER_NO_FREE_SURFACES,
];

/// `AMFComponent::SubmitInput` をリトライしながら呼ぶ
///
/// リトライ対象のステータスは 1ms 待ってから再試行し、それ以外のステータスは
/// そのまま返す。受理したか、解像度変化か、エラーかは呼び出し側が判定する。
///
/// # Errors
///
/// リトライの上限に達した場合にエラーを返す。
fn submit_input_with_retry(component: &Component, data: *mut AMFData) -> Result<AMF_RESULT, Error> {
    let mut last_status = AMF_RESULT::AMF_OK;
    for retry in 0..MAX_SUBMIT_RETRIES {
        let status = unsafe { component.submit_input(data) };
        last_status = status;

        if !SUBMIT_RETRY_STATUSES.contains(&status) {
            return Ok(status);
        }

        if retry > 0 && retry % 10 == 0 {
            tracing::trace!("AMFComponent::SubmitInput: retry={retry}, status={status:?}");
        }
        std::thread::sleep(SUBMIT_RETRY_INTERVAL);
    }

    Err(InternalError::new(format!(
        "AMFComponent::SubmitInput did not accept input after {MAX_SUBMIT_RETRIES} retries (last status: {last_status:?})"
    ))
    .into())
}

/// 出力をハンドラーへ通知する関数
type DeliverFn<U> = Box<dyn FnMut(Result<(Data, U), Error>) + Send>;

/// 保留中の入力がなくなったときに `AMF_EOF` を待たずに終了してよいかを返す関数
type DrainedFn<U> = Box<dyn Fn(&HashMap<i64, U>) -> bool + Send>;

/// ワーカースレッドの差分
///
/// エンコーダーとデコーダーは、ハンドラーへの通知と終了条件だけをクロージャで
/// 差し替えて同じループを使う。
pub(crate) struct WorkerConfig<U> {
    /// エラーメッセージとスレッド名に使うワーカーの役割
    pub role: &'static str,
    /// 出力をハンドラーへ通知する
    pub deliver: DeliverFn<U>,
    /// 保留中の入力がなくなったときに `AMF_EOF` を待たずに終了してよいかを返す
    pub drained: DrainedFn<U>,
}

impl<U> WorkerConfig<U> {
    /// 出力をハンドラーへ通知する
    fn deliver(&mut self, result: Result<(Data, U), Error>) {
        (self.deliver)(result);
    }

    /// 保留中の入力がなくなったときに `AMF_EOF` を待たずに終了してよいかを返す
    fn is_drained(&self, pending: &HashMap<i64, U>) -> bool {
        (self.drained)(pending)
    }
}

/// ワーカースレッドが受け取るコマンド
enum WorkerCommand<U> {
    /// 入力フレームの frame ID とユーザーデータを登録する
    RegisterUserData { frame_id: i64, user_data: U },
    /// `AMFComponent::Drain` を呼び、終了条件を満たすまで出力を回収する
    ///
    /// Drain に失敗した場合はエラーを返してコマンド待ちに戻る。回収中に
    /// 終端エラーを検出した場合はハンドラーへ通知してワーカーを終了する。
    WaitEof {
        /// 回収結果を通知するチャネル
        done: mpsc::SyncSender<Result<(), Error>>,
    },
    /// ワーカーを停止する
    ///
    /// 保持している入出力は通知せずに破棄する。
    Stop {
        /// 停止完了を通知するチャネル
        done: mpsc::SyncSender<()>,
    },
}

/// 共通ワーカースレッドを起動・制御する
///
/// Drop ではワーカーを停止して join する。`AMFComponent::Terminate` は
/// コンポーネントを所有する側が行う。
pub(crate) struct Worker<U: Send + 'static> {
    /// エラーメッセージに使うワーカーの役割
    role: &'static str,
    /// ワーカーへのコマンド送信。停止後は None
    cmd_tx: Option<mpsc::Sender<WorkerCommand<U>>>,
    /// ワーカースレッドのハンドル。join 後は None
    thread: Option<JoinHandle<()>>,
    /// 次に投入する入力フレームへ割り当てる frame ID
    next_frame_id: FrameIdGenerator,
}

impl<U: Send + 'static> Worker<U> {
    /// ワーカースレッドを起動する
    ///
    /// # Errors
    ///
    /// ワーカースレッドを起動できない場合にエラーを返す。
    pub fn spawn(component: Component, config: WorkerConfig<U>) -> Result<Self, Error> {
        let role = config.role;
        let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand<U>>();
        let worker_component = component.clone();
        let thread = std::thread::Builder::new()
            .name(format!("amf-{role}-worker"))
            .spawn(move || {
                run_worker(worker_component, config, cmd_rx);
            })
            .map_err(|source| {
                EnvironmentError::new(format!("failed to spawn {role} worker thread"), source)
            })?;

        Ok(Self {
            role,
            cmd_tx: Some(cmd_tx),
            thread: Some(thread),
            next_frame_id: FrameIdGenerator::new(),
        })
    }

    /// ワーカーがコマンドを受け付けられる状態かを返す
    pub fn is_running(&self) -> bool {
        self.cmd_tx.is_some()
    }

    /// 入力を `SubmitInput` に投入し、割り当てた frame ID と結果を返す
    ///
    /// 入力フレームのプロパティに frame ID を設定してから `SubmitInput` を呼ぶ。
    ///
    /// # Errors
    ///
    /// frame ID を発行できない場合、プロパティを設定できない場合、または
    /// `SubmitInput` がリトライ上限に達した場合にエラーを返す。
    pub fn submit_input(
        &mut self,
        component: &Component,
        data: &Data,
    ) -> Result<(i64, AMF_RESULT), Error> {
        let frame_id = self.next_frame_id.next()?;

        let property = data.property_storage()?;
        property.set_property_int64(FRAME_ID_PROPERTY, frame_id)?;

        // 入力オブジェクトを他のハンドルと共有したまま SubmitInput を呼ばないよう、
        // プロパティのハンドルを解放してから投入する。
        drop(property);

        let status = submit_input_with_retry(component, data.as_ptr())?;
        Ok((frame_id, status))
    }

    /// frame ID に対するユーザーデータを登録する
    ///
    /// `SubmitInput` が受理した入力フレームの frame ID と、そのフレームに
    /// 対応するユーザーデータをワーカーへ送る。ワーカーは出力の frame ID から
    /// ユーザーデータを引いてハンドラーへ通知する。
    ///
    /// # Errors
    ///
    /// ワーカーが停止している場合にエラーを返す。
    pub fn register_user_data(&self, frame_id: i64, user_data: U) -> Result<(), Error> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            InternalError::new(format!("{} worker thread is not running", self.role))
        })?;
        cmd_tx.send(WorkerCommand::RegisterUserData {
            frame_id,
            user_data,
        })?;
        Ok(())
    }

    /// ワーカーに Drain と出力の回収を依頼する
    ///
    /// 成功するとワーカーはコマンド待ちに戻る。
    ///
    /// # Errors
    ///
    /// Drain に失敗した場合、ワーカーが終了している場合、またはハンドラーが
    /// panic した場合にエラーを返す。エラーを返した後のワーカーの状態は
    /// 呼び出し側では保証しない。
    pub fn wait_eof(&mut self) -> Result<(), Error> {
        let Some(cmd_tx) = self.cmd_tx.as_ref() else {
            return Err(
                InternalError::new(format!("{} worker thread is not running", self.role)).into(),
            );
        };

        let (done_tx, done_rx) = mpsc::sync_channel(1);
        if cmd_tx
            .send(WorkerCommand::WaitEof { done: done_tx })
            .is_err()
        {
            // ワーカーは既に終了している。join で後始末する。
            self.cmd_tx = None;
            self.join()?;
            return Err(
                InternalError::new(format!("{} worker thread terminated", self.role)).into(),
            );
        }

        match done_rx.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                // ワーカーは終端エラーを通知して終了したか、panic した。
                // panic は join で検出する。
                self.cmd_tx = None;
                self.join()?;
                Err(InternalError::new(format!("{} worker thread terminated", self.role)).into())
            }
        }
    }

    /// 残りの出力を回収してワーカーを停止する
    ///
    /// `drain_required` が true の場合は `WaitEof` で `Drain` と出力の回収を行う。
    ///
    /// # Errors
    ///
    /// 既に停止している場合、残りの出力を回収できなかった場合、または
    /// ハンドラーが panic した場合にエラーを返す。エラーを返した後の操作は
    /// 呼び出し側では保証しない。
    pub fn finish(&mut self, drain_required: bool) -> Result<(), Error> {
        if !self.is_running() {
            return Err(
                InvalidStateError::new(format!("{} is already finished", self.role)).into(),
            );
        }

        if drain_required {
            self.wait_eof()?;
        }

        self.stop()
    }

    /// ワーカーを停止し、スレッドの終了を待つ
    ///
    /// # Errors
    ///
    /// ハンドラーが panic した場合、またはワーカーが応答せずに終了した場合に
    /// エラーを返す。
    pub fn stop(&mut self) -> Result<(), Error> {
        let Some(cmd_tx) = self.cmd_tx.take() else {
            return self.join();
        };

        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let worker_replied =
            cmd_tx.send(WorkerCommand::Stop { done: done_tx }).is_ok() && done_rx.recv().is_ok();
        let join_result = self.join();

        match (worker_replied, join_result) {
            (true, Ok(())) => Ok(()),
            (_, Err(error)) => Err(error),
            // Stop を送れなかった場合はワーカーの受信側が既に破棄されている。
            (false, Ok(())) => {
                Err(InternalError::new(format!("{} worker thread terminated", self.role)).into())
            }
        }
    }

    /// ワーカーに `Stop` を送って join する
    ///
    /// 戻り値を返せない後始末で使う。キューにあるコマンドはワーカーが処理する。
    /// ハンドラーの panic はログに記録する。
    pub fn shutdown(&mut self) {
        if let Err(error) = self.stop() {
            tracing::error!(role = self.role, "worker shutdown failed: {error}");
        }
    }

    /// ワーカースレッドを join する
    fn join(&mut self) -> Result<(), Error> {
        match self.thread.take() {
            Some(handle) => match handle.join() {
                Ok(()) => Ok(()),
                Err(_) => {
                    Err(InternalError::new(format!("{} worker thread panicked", self.role)).into())
                }
            },
            None => Ok(()),
        }
    }
}

impl<U: Send + 'static> Drop for Worker<U> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// ワーカースレッドのエントリポイント
///
/// コマンドを処理し、`QueryOutput` をポーリングして出力をハンドラーへ通知する。
/// `WaitEof` を受け取ると `Drain` を呼び、終了条件を満たすまで出力を回収する。
fn run_worker<U: Send + 'static>(
    component: Component,
    mut config: WorkerConfig<U>,
    cmd_rx: mpsc::Receiver<WorkerCommand<U>>,
) {
    let role = config.role;
    // frame ID から入力フレームのユーザーデータを引くための表。frame ID は
    // 入力ごとに一意なため、値の重複はない。
    let mut pending: HashMap<i64, U> = HashMap::new();
    // RegisterUserData コマンドの処理が出力の取得より遅れた場合に備え、対応する
    // ユーザーデータが登録されるまで出力を保持する。
    let mut output_buffer: VecDeque<Result<(Data, i64), Error>> = VecDeque::new();
    // WaitEof の完了通知。回収中は保持する
    let mut wait_eof: Option<mpsc::SyncSender<Result<(), Error>>> = None;
    // Drain が受理されたか
    let mut drain_accepted = false;

    loop {
        if let Some(done) = wait_eof.take() {
            if !drain_accepted {
                let result = component.drain();
                tracing::debug!(role, "worker: Drain result={result:?}");

                if result == AMF_RESULT::AMF_INPUT_FULL {
                    // 入力キューを空けるため、Drain を再試行する前に出力を回収する。
                    match drain_output(&mut output_buffer, &component, &config) {
                        Ok(DrainStatus::Eof) => {
                            finish_drain(&mut output_buffer, &mut pending, &mut config);
                            let _ = done.send(Ok(()));
                            continue;
                        }
                        Ok(DrainStatus::NotReady) => {
                            deliver_buffered(&mut output_buffer, &mut pending, &mut config);
                            // エンコーダーは保留中の入出力がなくなった時点で
                            // AMF_EOF を待たずに完了できる。
                            if output_buffer.is_empty() && config.is_drained(&pending) {
                                finish_drain(&mut output_buffer, &mut pending, &mut config);
                                let _ = done.send(Ok(()));
                                continue;
                            }
                        }
                        Err(error) => {
                            notify_terminal_error(
                                error,
                                role,
                                &mut output_buffer,
                                &mut pending,
                                &mut config,
                            );
                            return;
                        }
                    }

                    wait_eof = Some(done);
                    std::thread::sleep(WORKER_POLL_INTERVAL);
                    continue;
                }

                if result != AMF_RESULT::AMF_OK {
                    // Drain の失敗は呼び出し側へ返し、ワーカーはコマンド待ちに戻る。
                    let error: Error = AmfError::new(result, "AMFComponent::Drain").into();
                    let _ = done.send(Err(error));
                    continue;
                }
                drain_accepted = true;
            }

            match drain_output(&mut output_buffer, &component, &config) {
                Ok(status) => {
                    deliver_buffered(&mut output_buffer, &mut pending, &mut config);
                    if status == DrainStatus::Eof
                        || (output_buffer.is_empty() && config.is_drained(&pending))
                    {
                        finish_drain(&mut output_buffer, &mut pending, &mut config);
                        let _ = done.send(Ok(()));
                        drain_accepted = false;
                        continue;
                    }
                }
                Err(error) => {
                    notify_terminal_error(
                        error,
                        role,
                        &mut output_buffer,
                        &mut pending,
                        &mut config,
                    );
                    return;
                }
            }

            wait_eof = Some(done);
            std::thread::sleep(WORKER_POLL_INTERVAL);
            continue;
        }

        let cmd = if pending.is_empty() && output_buffer.is_empty() {
            match cmd_rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            }
        } else {
            // コマンドの到着を待つ間も出力を回収する。利用側が SubmitInput を
            // リトライしている間も、保留中の入力がある限り QueryOutput を呼ぶ。
            match cmd_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                Ok(cmd) => cmd,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    match drain_output(&mut output_buffer, &component, &config) {
                        Ok(_) => deliver_buffered(&mut output_buffer, &mut pending, &mut config),
                        Err(error) => {
                            notify_terminal_error(
                                error,
                                role,
                                &mut output_buffer,
                                &mut pending,
                                &mut config,
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
            WorkerCommand::RegisterUserData {
                frame_id,
                user_data,
            } => {
                pending.insert(frame_id, user_data);
                // コマンドを 1 つ処理した後にも出力を回収し、入力の投入が続いても
                // 出力の回収が滞らないようにする。
                match drain_output(&mut output_buffer, &component, &config) {
                    Ok(_) => deliver_buffered(&mut output_buffer, &mut pending, &mut config),
                    Err(error) => {
                        notify_terminal_error(
                            error,
                            role,
                            &mut output_buffer,
                            &mut pending,
                            &mut config,
                        );
                        break;
                    }
                }
            }
            WorkerCommand::WaitEof { done } => {
                wait_eof = Some(done);
                drain_accepted = false;
            }
            WorkerCommand::Stop { done } => {
                let _ = done.send(());
                break;
            }
        }
    }
}

/// `QueryOutput` から取得できた出力を `output_buffer` に追加する
///
/// 取得できる出力がなくなった場合は [`DrainStatus::NotReady`]、ドレインが
/// 完了した場合は [`DrainStatus::Eof`] を返す。終端エラーの場合は `Err` を返す。
fn drain_output<U: Send + 'static>(
    output_buffer: &mut VecDeque<Result<(Data, i64), Error>>,
    component: &Component,
    config: &WorkerConfig<U>,
) -> Result<DrainStatus, Error> {
    loop {
        let mut data: *mut AMFData = ptr::null_mut();
        let result = unsafe { component.query_output(&mut data) };
        tracing::trace!(role = config.role, "worker: QueryOutput result={result:?}");
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
                // 出力の所有権を Data に移す。作成に失敗した場合は Data が
                // 参照を解放する。
                let data = match unsafe { Data::from_raw(data, component.context()) } {
                    Ok(data) => data,
                    Err(error) => {
                        output_buffer.push_back(Err(error));
                        continue;
                    }
                };

                // frame ID は入出力の対応付けに使う。プロパティのハンドルは
                // 読み取り後すぐに解放し、出力を共有状態にしない。
                let frame_id = match (|| -> Result<i64, Error> {
                    let property = data.property_storage()?;
                    property.get_property_int64(FRAME_ID_PROPERTY)
                })() {
                    Ok(frame_id) => frame_id,
                    Err(error) => {
                        output_buffer.push_back(Err(error));
                        continue;
                    }
                };

                output_buffer.push_back(Ok((data, frame_id)));
            }
            QueryOutputKind::NoOutput => return Ok(DrainStatus::NotReady),
            QueryOutputKind::Eof => return Ok(DrainStatus::Eof),
            QueryOutputKind::TerminalError => {
                return Err(AmfError::new(result, "AMFComponent::QueryOutput").into());
            }
        }
    }
}

/// 対応する入力がある出力を、出力順にハンドラーへ通知する
///
/// 対応する入力がまだない出力は通知せずに保持する。出力の取得エラーは `Err` と
/// して通知する。
fn deliver_buffered<U: Send + 'static>(
    output_buffer: &mut VecDeque<Result<(Data, i64), Error>>,
    pending: &mut HashMap<i64, U>,
    config: &mut WorkerConfig<U>,
) {
    loop {
        let Some(result) = output_buffer.pop_front() else {
            return;
        };
        match result {
            Ok((data, frame_id)) => {
                let Some(user_data) = pending.remove(&frame_id) else {
                    output_buffer.push_front(Ok((data, frame_id)));
                    return;
                };
                config.deliver(Ok((data, user_data)));
            }
            Err(error) => config.deliver(Err(error)),
        }
    }
}

/// 保持している出力をすべてハンドラーへ通知する
///
/// 対応する入力がある出力は `Ok`、ない出力は `Err` として通知する。出力の取得
/// エラーも `Err` として通知する。
fn flush_buffered_outputs<U: Send + 'static>(
    output_buffer: &mut VecDeque<Result<(Data, i64), Error>>,
    pending: &mut HashMap<i64, U>,
    config: &mut WorkerConfig<U>,
) {
    while let Some(result) = output_buffer.pop_front() {
        match result {
            Ok((data, frame_id)) => match pending.remove(&frame_id) {
                Some(user_data) => config.deliver(Ok((data, user_data))),
                None => config.deliver(Err(InternalError::new(format!(
                    "the {} produced output that matches no submitted input (frame ID {frame_id})",
                    config.role
                ))
                .into())),
            },
            Err(error) => config.deliver(Err(error)),
        }
    }
}

/// 出力が得られなかった入力を `Err` として通知する
///
/// 通知順は入力順とする。
fn report_unmatched_inputs<U: Send + 'static>(
    pending: &mut HashMap<i64, U>,
    config: &mut WorkerConfig<U>,
) {
    let mut frame_ids: Vec<_> = pending.drain().map(|(frame_id, _)| frame_id).collect();
    frame_ids.sort_unstable();
    for frame_id in frame_ids {
        config.deliver(Err(InternalError::new(format!(
            "the {} did not produce output for the submitted input (frame ID {frame_id})",
            config.role
        ))
        .into()));
    }
}

/// 終端エラー発生時に、取得済みの出力、原因のエラー、出力が得られなかった入力の
/// 順にハンドラーへ通知する。
fn notify_terminal_error<U: Send + 'static>(
    error: Error,
    role: &'static str,
    output_buffer: &mut VecDeque<Result<(Data, i64), Error>>,
    pending: &mut HashMap<i64, U>,
    config: &mut WorkerConfig<U>,
) {
    flush_buffered_outputs(output_buffer, pending, config);
    config.deliver(Err(error));
    report_unmatched_inputs(pending, config);
    tracing::debug!(role, "worker: terminated by terminal error");
}

/// 保持している出力と、出力が得られなかった入力を通知して回収を完了する
fn finish_drain<U: Send + 'static>(
    output_buffer: &mut VecDeque<Result<(Data, i64), Error>>,
    pending: &mut HashMap<i64, U>,
    config: &mut WorkerConfig<U>,
) {
    flush_buffered_outputs(output_buffer, pending, config);
    report_unmatched_inputs(pending, config);
}
