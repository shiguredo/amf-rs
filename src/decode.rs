//! AMF ハードウェアデコーダー
//!
//! AMD GPU を使ったハードウェアビデオデコードを提供する。
//! H.264/AVC、H.265/HEVC、AV1 コーデックに対応する。

use std::sync::Arc;

use crate::AmfLibrary;
use crate::amf::{Buffer, Component, Context, Data, Surface};
use crate::error::{AmfError, Error, InvalidArgumentError, InvalidStateError};
use crate::sys::{self, AMF_RESULT};
use crate::types::{FrameFormat, MemoryType, SurfaceFormat};
use crate::worker::{DecoderStats, Worker, WorkerConfig, WorkerStats};

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

/// AMF ハードウェアデコーダー
///
/// デコード結果は、[`Decoder::new`] に渡したハンドラーへワーカースレッドから通知される。
///
/// [`Decoder::decode`] / [`Decoder::finish`] がエラーを返した場合、その後は
/// `Drop` 以外の操作を保証しない。`Drop` は安全に呼び出せる。
///
/// # Drop
///
/// [`Decoder::finish`] を呼ばずに drop した場合、drop はワーカーが停止するまで
/// 待つ。停止処理の途中で、すでに [`Decoder::decode`] が成功したフレームの出力が
/// 通知されることがある。drop は、すべてのフレームの出力が通知されることを
/// 保証しない。
pub struct Decoder<H: DecodeHandler> {
    component: Component,
    context: Context,
    /// ワーカースレッドの制御
    worker: Worker<H::UserData>,
    /// 統計値
    ///
    /// ワーカースレッドと共有し、デコード処理の進行に伴って更新する。
    stats: Arc<DecoderStats>,
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
    pub fn new(context: Context, config: DecoderConfig, mut handler: H) -> Result<Self, Error> {
        let lib = AmfLibrary::instance();

        let component_id = match config.codec {
            DecoderCodec::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
            DecoderCodec::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
            DecoderCodec::Av1 => sys::str::AMFVideoDecoderHW_AV1,
        };

        let component = lib.create_component(&context, component_id)?;

        // デコーダーを初期化する (解像度は 0,0 でストリームから自動検出)
        let result = component.init(SurfaceFormat::from(FrameFormat::Nv12), 0, 0);
        AmfError::check(result, "AMFComponent::Init")?;

        // 統計値はワーカースレッドと共有する。
        let stats = Arc::new(DecoderStats::default());

        let deliver_stats = Arc::clone(&stats);
        let worker_stats = WorkerStats::Decoder(Arc::clone(&stats));
        let worker = Worker::spawn(
            component.clone(),
            WorkerConfig {
                role: "decoder",
                deliver: Box::new(move |result| {
                    let result: Result<DecodedFrame<H::UserData>, H::Error> = result
                        .and_then(|(data, user_data)| {
                            let surface = data.to_surface()?;
                            surface.convert(MemoryType::Host)?;
                            Ok(DecodedFrame { surface, user_data })
                        })
                        .map_err(Into::into);
                    match &result {
                        Ok(_) => deliver_stats.total_output_frame_count.inc(),
                        Err(_) => deliver_stats.total_error_count.inc(),
                    }
                    handler.on_decoded(result);
                }),
                stats: worker_stats,
            },
        )?;

        Ok(Self {
            component,
            context,
            worker,
            stats,
            drain_required: false,
            initialized: true,
        })
    }

    /// このデコーダーが使用する `Context` を返す
    pub fn context(&self) -> Context {
        self.context.clone()
    }

    /// デコーダーの統計値を取得する
    ///
    /// 値はデコード処理の進行に伴って更新される。各フィールドの意味は
    /// [`DecoderStats`] を参照すること。
    pub fn stats(&self) -> &DecoderStats {
        &self.stats
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
        if !self.worker.is_running() {
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

        tracing::trace!("Decoder::decode");

        let data = buffer.into_data()?;
        let (frame_id, status) = self.worker.submit_input(&self.component, &data)?;
        match status {
            AMF_RESULT::AMF_OK | AMF_RESULT::AMF_NEED_MORE_INPUT => {
                // AMF が入力を処理したため、終了時には Drain が必要になる。
                self.drain_required = true;
                self.register_accepted_input(frame_id, user_data)?;
                Ok(())
            }
            AMF_RESULT::AMF_RESOLUTION_CHANGED => {
                // AMF は解像度の変化を検出すると、このバッファを受理せずに
                // Drain / Terminate / Init によるデコーダーの再作成を要求する。
                // 旧解像度の出力を通知させてから AMF コンポーネントを新しい解像度で
                // 作り直し、バッファを再投入する。
                tracing::debug!("Decoder::decode: resolution changed");
                self.reinitialize_after_resolution_change(data, user_data)
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
            status => Err(AmfError::new(status, "AMFComponent::SubmitInput").into()),
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
        data: Data,
        user_data: H::UserData,
    ) -> Result<(), Error> {
        // 旧解像度の出力をすべて通知させ、ワーカーをコマンド待ちに戻す。
        // Drain と AMF_EOF 待ちはワーカーが行う。
        self.worker.wait_eof()?;
        self.drain_required = false;

        // AMF_RESOLUTION_CHANGED は Drain / Terminate / Init を要求する
        // (AMF SDK v1.5.2 の include/core/Result.h)。AMF_VIDEO_DECODER_CURRENT_SIZE は
        // 現在の解像度を返す (components/VideoDecoderUVD.h)。AMF の実装によって
        // 変わり得る。
        let property_storage = self.component.property_storage()?;
        let (width, height) =
            property_storage.get_property_size(sys::str::AMF_VIDEO_DECODER_CURRENT_SIZE)?;

        // コンポーネントを作り直す。
        let result = self.component.terminate();
        AmfError::check(result, "AMFComponent::Terminate")?;
        self.initialized = false;
        self.drain_required = false;
        let result = self
            .component
            .init(SurfaceFormat::from(FrameFormat::Nv12), width, height);
        AmfError::check(result, "AMFComponent::Init")?;
        self.initialized = true;

        // AMF に受理されなかったバッファを新しい解像度のデコーダーへ再投入する。
        let (frame_id, status) = self.worker.submit_input(&self.component, &data)?;
        match status {
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
            status => {
                return Err(AmfError::new(status, "AMFComponent::SubmitInput").into());
            }
        }

        // 再投入したフレームをワーカーへ登録し、通常ループを再開させる。
        self.register_accepted_input(frame_id, user_data)?;
        Ok(())
    }

    /// 受理したフレームをワーカーへ登録し、統計値を更新する
    ///
    /// # Errors
    ///
    /// ワーカーが停止している場合にエラーを返す。この場合、統計値は更新しない。
    fn register_accepted_input(
        &mut self,
        frame_id: i64,
        user_data: H::UserData,
    ) -> Result<(), Error> {
        // ワーカーは登録を処理した直後に対応する出力を通知し得るため、登録より
        // 先に in-flight を計上する。登録に失敗した場合はこのフレームの出力が
        // 通知されないため、計上を取り消す。
        self.stats.in_flight_frames.inc();
        if let Err(error) = self.worker.register_user_data(frame_id, user_data) {
            self.stats.in_flight_frames.dec();
            return Err(error);
        }
        self.stats.total_decode_count.inc();
        Ok(())
    }

    /// 残りのフレームを出力させ、ハンドラーへの通知が完了するまで待つ。
    ///
    /// 出力の回収中に発生した終端エラーはハンドラーへ通知される。
    ///
    /// # Errors
    ///
    /// `finish` 済みの場合、残りの出力を回収できなかった場合、または
    /// ハンドラーが panic した場合にエラーを返す。エラーを返した後の操作は
    /// `Drop` 以外を保証しない。
    pub fn finish(&mut self) -> Result<(), Error> {
        self.worker.finish(self.drain_required)?;
        self.drain_required = false;
        Ok(())
    }
}

// SAFETY:
// Drop 内でのみ component を解放する。ワーカースレッドは Drop より先に停止させる。
impl<H: DecodeHandler> Drop for Decoder<H> {
    fn drop(&mut self) {
        // ワーカースレッドに Stop を送り、キューにあるコマンドを処理してから停止する。
        // Stop を処理した時点で残っている入出力は通知せずに破棄される。
        self.worker.shutdown();

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
