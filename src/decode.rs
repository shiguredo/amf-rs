//! AMF ハードウェアデコーダー
//!
//! AMD GPU を使ったハードウェアビデオデコードを提供する。
//! H.264/AVC、H.265/HEVC、AV1、VP9 コーデックに対応する。

use std::sync::Arc;

use crate::AmfLibrary;
use crate::amf::{Buffer, Component, Context, Data, PropertyStorage, Surface};
use crate::error::{AmfError, Error, InvalidArgumentError, InvalidStateError};
use crate::sys::{self, AMF_RESULT, amf_int64};
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
    /// VP9
    ///
    /// 実機で確認済みなのは Profile 0 (8bit 4:2:0) である。10bit (Profile 2) は
    /// 出力フォーマットが NV12 固定のため未検証である。
    ///
    /// superframe は 1 フレームずつ分割して投入する必要がある。不可視フレーム
    /// (`show_frame=0`) や `show_existing_frame` を含むストリームは、入力と出力の
    /// 対応付けができないため、[`DecodeHandler`] にエラーが通知される。
    Vp9,
}

/// デコーダーのリオーダーモード
///
/// デコーダーが出力フレームの提示順を待ち合わせるために保持する DPB の遅延を
/// 指定する。AMF の `AMF_VIDEO_DECODER_MODE_ENUM` に対応する
/// (AMF SDK v1.5.2 の `components/VideoDecoderUVD.h`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderReorderMode {
    /// ストリームの SPS が示す参照フレーム数 + 1 を DPB の遅延にする
    Regular,
    /// プロファイルに基づき最大 16 フレームを DPB の遅延にする
    Compliant,
    /// DPB の遅延を 0 にする
    ///
    /// フレームの並べ替えを伴わないストリームを前提とする。B フレームと P フレームは、
    /// 並べ替えを伴わない場合に限り含められる。並べ替えを伴うストリームを投入した
    /// 場合の出力順は保証しない。このモードでは提示順の復元を行わない。
    LowLatency,
}

impl DecoderReorderMode {
    /// AMF の `AMF_VIDEO_DECODER_MODE_ENUM` の値を返す
    fn to_amf(self) -> sys::AMF_VIDEO_DECODER_MODE_ENUM {
        match self {
            Self::Regular => sys::AMF_VIDEO_DECODER_MODE_ENUM_AMF_VIDEO_DECODER_MODE_REGULAR,
            Self::Compliant => sys::AMF_VIDEO_DECODER_MODE_ENUM_AMF_VIDEO_DECODER_MODE_COMPLIANT,
            Self::LowLatency => sys::AMF_VIDEO_DECODER_MODE_ENUM_AMF_VIDEO_DECODER_MODE_LOW_LATENCY,
        }
    }
}

/// デコーダー設定
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    /// デコードするコーデック
    ///
    /// 出力フォーマットは NV12 に固定される。
    pub codec: DecoderCodec,
    /// 低遅延デコードを有効にする
    ///
    /// `Some(false)` は通常のデコードを明示する。`None` は AMF の既定値を使う。
    /// 出力フレームの順序は [`DecoderConfig::reorder_mode`] で決まる。
    ///
    /// AMF の `AMF_VIDEO_DECODER_LOW_LATENCY` に対応する。
    pub low_latency: Option<bool>,
    /// DPB のリオーダーモード
    ///
    /// デコーダーが出力フレームの提示順を待ち合わせるために保持するフレーム数を
    /// 決める。`None` は AMF の既定値を使う。
    pub reorder_mode: Option<DecoderReorderMode>,
    /// デコード用サーフェスプールのサイズ
    ///
    /// DPB が保持するサーフェス数と、提示用に保持するサーフェス数の合計を
    /// 指定する。`Some(0)` は AMF に DPB サイズ + 5 を計算させる。`None` は
    /// AMF の既定値を使う。
    ///
    /// 値が小さすぎる場合、出力サーフェスが不足して `decode` がエラーになる。
    pub surface_pool_size: Option<u32>,
}

impl DecoderConfig {
    /// コーデックだけを指定してデコーダー設定を作成する
    ///
    /// 遅延とサーフェスプールの各項目は `None` に設定する。
    pub fn new(codec: DecoderCodec) -> Self {
        Self {
            codec,
            low_latency: None,
            reorder_mode: None,
            surface_pool_size: None,
        }
    }
}

/// デコード済みフレーム
#[derive(Debug)]
pub struct DecodedFrame<T> {
    surface: Surface,
    user_data: T,
}

impl<T> DecodedFrame<T> {
    /// デコード後の Surface
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
    /// 初期化時に適用した設定
    ///
    /// 解像度変化で AMF コンポーネントを作り直すときに、同じ設定を再適用するために
    /// 保持する。
    config: DecoderConfig,
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
    /// 指定したコーデックや設定に対応していない場合も含む。
    pub fn new(context: Context, config: DecoderConfig, mut handler: H) -> Result<Self, Error> {
        let lib = AmfLibrary::instance();

        let component_id = match config.codec {
            DecoderCodec::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
            DecoderCodec::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
            DecoderCodec::Av1 => sys::str::AMFVideoDecoderHW_AV1,
            DecoderCodec::Vp9 => sys::str::AMFVideoDecoderHW_VP9,
        };

        let component = lib.create_component(&context, component_id)?;

        // 解像度は 0,0 でストリームから自動検出させる。デコーダー固有の設定は
        // Init より前に設定する必要がある。
        let prop = component.property_storage()?;
        Self::set_properties(&prop, &config)?;

        // デコーダーを初期化する
        let result = component.init(SurfaceFormat::from(FrameFormat::Nv12), 0, 0);
        AmfError::check(result, "AMFComponent::Init")?;

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
            config,
            worker,
            stats,
            drain_required: false,
            initialized: true,
        })
    }

    /// デコーダー固有のプロパティを設定する
    ///
    /// `Some` の項目だけを設定し、`None` の項目のプロパティは変更しない。
    ///
    /// # Errors
    ///
    /// AMF が指定した値を受け付けない場合にエラーを返す。
    fn set_properties(prop: &PropertyStorage, config: &DecoderConfig) -> Result<(), Error> {
        if let Some(low_latency) = config.low_latency {
            prop.set_property_bool(sys::str::AMF_VIDEO_DECODER_LOW_LATENCY, low_latency)?;
        }
        if let Some(reorder_mode) = config.reorder_mode {
            prop.set_property_int64(
                sys::str::AMF_VIDEO_DECODER_REORDER_MODE,
                reorder_mode.to_amf() as amf_int64,
            )?;
        }
        if let Some(surface_pool_size) = config.surface_pool_size {
            prop.set_property_int64(
                sys::str::AMF_VIDEO_DECODER_SURFACE_POOL_SIZE,
                surface_pool_size as amf_int64,
            )?;
        }
        Ok(())
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

        // Init より前に、作成時と同じデコーダー固有の設定を再適用する。
        let prop = self.component.property_storage()?;
        Self::set_properties(&prop, &self.config)?;

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
    fn register_accepted_input(&self, frame_id: i64, user_data: H::UserData) -> Result<(), Error> {
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
