//! AMD AMF (Advanced Media Framework) の Rust バインディング
//!
//! AMD GPU を使ったハードウェアアクセラレーションによるビデオエンコード/デコードを提供する。
//! AMF ランタイムライブラリ (libamfrt64.so.1) はドライバーに同梱されており、
//! dlopen で動的にロードされる。
//!
//! # エンコードの例
//!
//! ```no_run
//! use std::sync::mpsc;
//!
//! use shiguredo_amf::{
//!     AmfLibrary, CodecConfig, EncodeOptions, EncodedFrame, Encoder, EncoderConfig,
//!     FnEncodeHandler, FrameFormat, H264EncoderConfig, RateControlMode, frame_type,
//! };
//!
//! let context = AmfLibrary::instance()
//!     .create_context()
//!     .expect("failed to create AMF context");
//!
//! let mut config = EncoderConfig::new(
//!     CodecConfig::H264(H264EncoderConfig::default()),
//!     1920,
//!     1080,
//!     FrameFormat::Nv12,
//!     30,
//!     1,
//!     RateControlMode::Cbr,
//! );
//! config.target_kbps = Some(5_000);
//!
//! // エンコード結果はワーカースレッドからチャネルで受け取る
//! let (tx, rx) = mpsc::channel();
//! let mut encoder = Encoder::new(
//!     context,
//!     config,
//!     FnEncodeHandler::new(
//!         move |result: Result<EncodedFrame<()>, shiguredo_amf::Error>| {
//!             tx.send(result).expect("failed to send encoded frame");
//!         },
//!     ),
//! )
//! .expect("failed to create encoder");
//!
//! // Surface を確保し、プレーンにフレームデータを書き込む (書き込み処理は省略)
//! let surface = encoder.alloc_surface().expect("failed to allocate surface");
//! // ... ここで surface にフレームデータを書き込む ...
//!
//! encoder
//!     .encode(
//!         surface,
//!         EncodeOptions {
//!             frame_type: frame_type::IDR,
//!         },
//!         (),
//!     )
//!     .expect("failed to encode frame");
//!
//! // 残りのフレームを出力させ、ハンドラーへの通知が完了するまで待つ
//! encoder.finish().expect("failed to finish encoding");
//!
//! for result in rx.try_iter() {
//!     let frame = result.expect("failed to receive encoded frame");
//!     println!("encoded {} bytes", frame.buffer().get_size());
//! }
//! ```

#![warn(missing_docs)]

pub mod amf;
mod codec_info;
mod decode;
mod dl;
mod encode;
mod error;
mod query_output;
mod stats;
mod sys;
mod types;
mod worker;

use std::cell::UnsafeCell;
use std::path::Path;
use std::ptr;
use std::sync::{Arc, LazyLock, MutexGuard};

use amf::{AmfObjectState, Component, Context};
use error::require_vtbl_fn;
use sys::{AMF_DLL_NAME, AMF_FULL_VERSION, AMFFactory, AmfInitFn, AmfQueryVersionFn, amf_uint64};

pub use codec_info::{
    AccelerationType, CodecInfo, DecodingInfo, EncodingInfo, IoCapabilities, MemoryTypeCapability,
    Property, SurfaceFormatCapability, Value, VideoCodecType, VideoProfile, supported_codecs,
};
pub use decode::{
    DecodeHandler, DecodedFrame, Decoder, DecoderCodec, DecoderConfig, DecoderReorderMode,
    FnDecodeHandler,
};
pub use encode::{
    Av1EncoderConfig, CodecConfig, EncodeHandler, EncodeOptions, EncodedFrame, Encoder,
    EncoderConfig, EncoderUsage, FnEncodeHandler, H264EncoderConfig, HevcEncoderConfig,
    PictureType, QpRange, QualityPreset, RateControlMode, ReconfigureParams, frame_type,
};
pub use error::{
    AmfError, EnvironmentError, Error, InternalError, InvalidArgumentError, InvalidStateError,
};
pub use stats::{Counter, Gauge};
pub use types::{
    Av1Profile, DataType, FrameFormat, H264Profile, HevcProfile, MemoryType, PlaneType,
    SurfaceFormat,
};
pub use worker::{DecoderStats, EncoderStats};

/// ビルド時の AMF バージョン文字列
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// AMF ランタイムのバージョン
///
/// `major.minor.release.build` の 4 成分からなる。`Display` はこの順に
/// ドットで区切って表示する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmfVersion {
    /// メジャーバージョン
    pub major: u16,
    /// マイナーバージョン
    pub minor: u16,
    /// リリース番号
    pub release: u16,
    /// ビルド番号
    pub build: u16,
}

impl std::fmt::Display for AmfVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.major, self.minor, self.release, self.build
        )
    }
}

/// FFI モジュール (内部用、semver 保証対象外)
#[doc(hidden)]
pub mod ffi {
    pub use crate::sys::*;
}

/// プロセス全体で単一の AmfLibrary インスタンス
///
/// AMFInit が返す AMFFactory はプロセスシングルトンであるため、
/// AmfLibrary も 1 つだけ存在すればよい。
static AMF_LIBRARY: LazyLock<AmfLibrary> = LazyLock::new(AmfLibrary::new);

struct AmfLibraryInner {
    lib: dl::DynLib,
    factory: *mut AMFFactory,
}

/// AMF ランタイムライブラリのラッパー
///
/// dlopen で libamfrt64.so.1 をロードし、AMFFactory を取得する。
/// プロセス全体で単一のインスタンスのみ存在し、
/// [`AmfLibrary::instance()`] で取得する。AMF ランタイムのロードは、最初に
/// AMF ランタイムを必要とする呼び出し時に行う。
pub struct AmfLibrary {
    /// プロセス全体で共有するロック
    ///
    /// AMF のインターフェースはほとんどがスレッドセーフではないため、
    /// AMF ランタイムのロード、Context 作成、Context / Component の操作を
    /// 直列化する。
    state: Arc<AmfObjectState>,
    /// AMF ランタイムの状態
    ///
    /// `state` のロック保持中にのみアクセスすること。
    inner: UnsafeCell<Option<AmfLibraryInner>>,
}

unsafe impl Send for AmfLibrary {}
unsafe impl Sync for AmfLibrary {}

impl AmfLibrary {
    fn new() -> Self {
        Self {
            state: AmfObjectState::new(),
            inner: UnsafeCell::new(None),
        }
    }

    /// プロセス全体で共有する AMF ライブラリインスタンスを返す
    ///
    /// AMF ランタイムライブラリは、[`AmfLibrary::create_context`] や
    /// [`AmfLibrary::query_version`] の初回呼び出し時にロードされる。
    pub fn instance() -> &'static AmfLibrary {
        &AMF_LIBRARY
    }

    /// プロセス全体で単一の AmfLibraryInner インスタンスを返す
    ///
    /// 初回呼び出し時に AMF ランタイムをロードし、初期化に成功した後は
    /// キャッシュされたインスタンスを返す。ロードに失敗した場合はその失敗を
    /// キャッシュせず、次の呼び出しで再度ロードを試みる。
    ///
    /// `_guard` は `self.state` のロックを保持していることを示すトークンである。
    ///
    /// # Errors
    ///
    /// AMF ランタイムをロードできない場合、または AMF の初期化に失敗した場合に
    /// エラーを返す。
    fn ensure_inner<'a>(
        &'a self,
        _guard: &MutexGuard<'a, ()>,
    ) -> Result<&'a AmfLibraryInner, Error> {
        // SAFETY: 呼び出し元は `self.state` のロックを保持しており、`inner` への
        // アクセスはすべてこのロックの下で行う。
        let inner = unsafe { &mut *self.inner.get() };
        if inner.is_none() {
            let lib = dl::DynLib::open(Path::new(AMF_DLL_NAME))?;

            let amf_init: AmfInitFn = unsafe { lib.get(b"AMFInit") }?;

            let mut factory: *mut AMFFactory = ptr::null_mut();
            let result = unsafe { amf_init(AMF_FULL_VERSION, &mut factory) };
            AmfError::check(result, "AMFInit")?;

            if factory.is_null() {
                return Err(InternalError::new("AMFInit returned null factory").into());
            }

            // 初期化に成功した場合だけキャッシュする。失敗時は `inner` を
            // `None` のままにし、次の呼び出しで再試行できるようにする。
            *inner = Some(AmfLibraryInner { lib, factory });
        }
        Ok(inner.as_ref().expect("AmfLibraryInner must be initialized"))
    }

    /// AMF ランタイムのバージョンを問い合わせる
    ///
    /// # Errors
    ///
    /// AMF ランタイムをロードできない場合、またはバージョンの取得に失敗した場合に
    /// エラーを返す。
    pub fn query_version(&self) -> Result<AmfVersion, Error> {
        let guard = self.state.lock();
        let inner = self.ensure_inner(&guard)?;
        let amf_query_version: AmfQueryVersionFn = unsafe { inner.lib.get(b"AMFQueryVersion") }?;

        let mut version: amf_uint64 = 0;
        let result = unsafe { amf_query_version(&mut version) };
        AmfError::check(result, "AMFQueryVersion")?;

        let major = ((version >> 48) & 0xFFFF) as u16;
        let minor = ((version >> 32) & 0xFFFF) as u16;
        let release = ((version >> 16) & 0xFFFF) as u16;
        let build = (version & 0xFFFF) as u16;
        Ok(AmfVersion {
            major,
            minor,
            release,
            build,
        })
    }

    /// 新しい AMFContext を作成し、既定の Vulkan デバイスで初期化する
    ///
    /// # Errors
    /// AMF ランタイムを初期化できない場合にエラーを返す。
    pub fn create_context(&self) -> Result<Context, Error> {
        // Context::init_vulkan も state のロックを取得するため、Context を作成したら
        // ロックを解放してから Vulkan デバイスを初期化する。
        let context = {
            let guard = self.state.lock();
            let inner = self.ensure_inner(&guard)?;
            let mut context: *mut sys::AMFContext = ptr::null_mut();
            let result = unsafe {
                let vtbl = &*(*inner.factory).pVtbl;
                require_vtbl_fn(vtbl.CreateContext, "AMFFactory::CreateContext")?(
                    inner.factory,
                    &mut context,
                )
            };
            AmfError::check(result, "AMFFactory::CreateContext")?;

            if context.is_null() {
                return Err(InternalError::new("CreateContext returned null").into());
            }

            unsafe { Context::from_raw_with_state(context, Arc::clone(&self.state)) }?
        };

        unsafe { context.init_vulkan(ptr::null_mut()) }?;
        Ok(context)
    }

    /// AMFComponent を作成する
    pub(crate) fn create_component(
        &self,
        context: &Context,
        component_id: &str,
    ) -> Result<Component, Error> {
        let component = {
            let guard = self.state.lock();
            let inner = self.ensure_inner(&guard)?;
            let component_id_w = sys::to_wstring(component_id);
            let mut component: *mut sys::AMFComponent = ptr::null_mut();
            let result = unsafe {
                let vtbl = &*(*inner.factory).pVtbl;
                require_vtbl_fn(vtbl.CreateComponent, "AMFFactory::CreateComponent")?(
                    inner.factory,
                    context.as_ptr(),
                    component_id_w.as_ptr(),
                    &mut component,
                )
            };
            AmfError::check(result, "AMFFactory::CreateComponent")?;
            if component.is_null() {
                return Err(InternalError::new("CreateComponent returned null").into());
            }
            component
        };

        unsafe {
            Component::from_raw_with_state(component, Arc::clone(&self.state), context.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    /// 複数の Context を並列に作成・初期化しても SIGSEGV しないことを確認する
    #[test]
    fn test_parallel_create_context() {
        const CONTEXT_COUNT: usize = 64;

        let lib = AmfLibrary::instance();
        let barrier = Arc::new(Barrier::new(CONTEXT_COUNT));
        let contexts = std::thread::scope(|scope| {
            let handles = (0..CONTEXT_COUNT)
                .map(|_| {
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        lib.create_context().expect("Context の作成に失敗")
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("スレッドの join に失敗"))
                .collect::<Vec<_>>()
        });

        for (i, context) in contexts.iter().enumerate() {
            for other in contexts.iter().skip(i + 1) {
                assert!(
                    context.as_ptr() != other.as_ptr(),
                    "同じ Context が複数作成された"
                );
            }
        }
    }
}
