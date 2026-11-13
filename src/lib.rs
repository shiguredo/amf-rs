//! AMD AMF (Advanced Media Framework) の Rust バインディング
//!
//! AMD GPU を使ったハードウェアアクセラレーションによるビデオエンコード/デコードを提供する。
//! AMF ランタイムライブラリ (libamfrt64.so.1) はドライバーに同梱されており、
//! dlopen で動的にロードされる。

pub mod amf;
mod codec_info;
mod decode;
mod dl;
mod encode;
mod error;
mod sys;

use std::cell::UnsafeCell;
use std::path::Path;
use std::ptr;
use std::sync::{Arc, LazyLock, MutexGuard};

use amf::{AmfObjectState, Component, Context};
use error::require_vtbl_fn;
use sys::{AMF_DLL_NAME, AMF_FULL_VERSION, AMFFactory, AMFInit_Fn, AMFQueryVersion_Fn, amf_uint64};

pub use codec_info::{
    Av1EncodingProfile, CodecInfo, DecodingInfo, EncodingInfo, EncodingProfiles,
    H264EncodingProfile, HevcEncodingProfile, VideoCodecType, supported_codecs,
};
pub use decode::{
    DecodeHandler, DecodedFrame, Decoder, DecoderCodec, DecoderConfig, FnDecodeHandler,
};
pub use encode::{
    Av1EncoderConfig, Av1Profile, CodecConfig, EncodeHandler, EncodeOptions, EncodedFrame, Encoder,
    EncoderConfig, FnEncodeHandler, FrameFormat, H264EncoderConfig, H264Profile, HevcEncoderConfig,
    HevcProfile, PictureType, RateControlMode, ReconfigureParams, frame_type,
};
pub use error::Error;

/// ビルド時の AMF バージョン文字列
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

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
/// [`AmfLibrary::instance()`] で取得する。
pub struct AmfLibrary {
    /// プロセス全体で共有するロック
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

    pub fn instance() -> &'static AmfLibrary {
        &AMF_LIBRARY
    }

    /// プロセス全体で単一の AmfLibraryInner インスタンスを返す
    ///
    /// 初回呼び出し時に AMF ランタイムをロードし、2 回目以降はキャッシュされた
    /// インスタンスを返す。
    ///
    /// `_guard` は `self.state` のロックを保持していることを示すトークンである。
    fn ensure_inner<'a>(
        &'a self,
        _guard: &MutexGuard<'a, ()>,
    ) -> Result<&'a AmfLibraryInner, Error> {
        // SAFETY: 呼び出し元は `self.state` のロックを保持しており、`inner` への
        // アクセスはすべてこのロックの下で行う。
        let inner = unsafe { &mut *self.inner.get() };
        if inner.is_none() {
            let lib = dl::DynLib::open(Path::new(AMF_DLL_NAME)).map_err(|e| {
                Error::new_custom(
                    "AmfLibrary::ensure_inner",
                    &format!("failed to load {AMF_DLL_NAME}: {e}"),
                )
            })?;

            let amf_init: AMFInit_Fn = unsafe { lib.get(b"AMFInit") }.map_err(|e| {
                Error::new_custom(
                    "AmfLibrary::ensure_inner",
                    &format!("failed to find AMFInit: {e}"),
                )
            })?;

            let mut factory: *mut AMFFactory = ptr::null_mut();
            let result = unsafe { amf_init(AMF_FULL_VERSION, &mut factory) };
            Error::check(result, "AMFInit")?;

            if factory.is_null() {
                return Err(Error::new_custom(
                    "AmfLibrary::ensure_inner",
                    "AMFInit returned null factory",
                ));
            }

            *inner = Some(AmfLibraryInner { lib, factory });
        }
        Ok(inner.as_ref().expect("AmfLibraryInner must be initialized"))
    }

    /// AMF ランタイムのバージョンを問い合わせる
    pub fn query_version(&self) -> Result<(u16, u16, u16, u16), Error> {
        let guard = self.state.lock();
        let inner = self.ensure_inner(&guard)?;
        let amf_query_version: AMFQueryVersion_Fn = unsafe { inner.lib.get(b"AMFQueryVersion") }
            .map_err(|e| {
                Error::new_custom(
                    "AmfLibrary::query_version",
                    &format!("failed to find AMFQueryVersion: {e}"),
                )
            })?;

        let mut version: amf_uint64 = 0;
        let result = unsafe { amf_query_version(&mut version) };
        Error::check(result, "AMFQueryVersion")?;

        let major = ((version >> 48) & 0xFFFF) as u16;
        let minor = ((version >> 32) & 0xFFFF) as u16;
        let release = ((version >> 16) & 0xFFFF) as u16;
        let build = (version & 0xFFFF) as u16;
        Ok((major, minor, release, build))
    }

    /// 新しい AMFContext を作成し、既定の Vulkan デバイスで初期化する
    ///
    /// # Errors
    /// AMF ランタイムのロード、Context の作成、または Vulkan デバイスの初期化に
    /// 失敗した場合にエラーを返す。
    pub fn create_context(&self) -> Result<Context, Error> {
        // Context::init_vulkan も state のロックを取得するため、Context を作成したら
        // ロックを解放してから Vulkan デバイスを初期化する。
        let context = {
            let guard = self.state.lock();
            let inner = self.ensure_inner(&guard)?;
            let mut context: *mut sys::AMFContext = ptr::null_mut();
            let result = unsafe {
                let vtbl = &*(*inner.factory).pVtbl;
                require_vtbl_fn(vtbl.CreateContext, "CreateContext")?(inner.factory, &mut context)
            };
            Error::check(result, "AMFFactory::CreateContext")?;

            if context.is_null() {
                return Err(Error::new_custom(
                    "AmfLibrary::create_context",
                    "CreateContext returned null",
                ));
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
                require_vtbl_fn(vtbl.CreateComponent, "CreateComponent")?(
                    inner.factory,
                    context.as_ptr(),
                    component_id_w.as_ptr(),
                    &mut component,
                )
            };
            Error::check(result, "AMFFactory::CreateComponent")?;
            if component.is_null() {
                return Err(Error::new_custom(
                    "AmfLibrary::create_component",
                    "CreateComponent returned null",
                ));
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
