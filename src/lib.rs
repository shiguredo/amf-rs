//! AMD AMF (Advanced Media Framework) の Rust バインディング
//!
//! AMD GPU を使ったハードウェアアクセラレーションによるビデオエンコード/デコードを提供する。
//! AMF ランタイムライブラリ (libamfrt64.so.1) はドライバーに同梱されており、
//! dlopen で動的にロードされる。

mod codec_info;
mod decode;
mod dl;
mod encode;
mod error;
mod sys;

use std::path::Path;
use std::ptr;

use std::ffi::c_void;

use sys::{
    AMF_DLL_NAME, AMF_FULL_VERSION, AMFContext, AMFContext1, AMFFactory, AMFInit_Fn,
    AMFQueryVersion_Fn, IID_AMF_CONTEXT1, amf_uint64,
};

pub use codec_info::{
    Av1EncodingProfile, CodecInfo, DecodingInfo, EncodingInfo, EncodingProfiles,
    H264EncodingProfile, HevcEncodingProfile, VideoCodecType, supported_codecs,
};
pub use decode::{DecodedFrame, Decoder, DecoderCodec, DecoderConfig};
pub use encode::{
    Av1EncoderConfig, Av1Profile, CodecConfig, EncodeOptions, EncodedFrame, Encoder, EncoderConfig,
    FrameFormat, H264EncoderConfig, H264Profile, HevcEncoderConfig, HevcProfile, PictureType,
    RateControlMode, frame_type,
};
pub use error::Error;

/// ビルド時の AMF バージョン文字列
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// FFI モジュール (内部用、semver 保証対象外)
#[doc(hidden)]
pub mod ffi {
    pub use crate::sys::*;
}

/// AMF ランタイムライブラリのラッパー
///
/// dlopen で libamfrt64.so.1 をロードし、AMFFactory を取得する。
///
/// AMFFactory は AMFInit が返すグローバルシングルトンであり、
/// ライブラリ (DynLib) のアンロード時に自動解放されるため
/// 明示的な Release は不要。Drop は DynLib 側で dlclose を呼ぶ。
pub struct AmfLibrary {
    _lib: dl::DynLib,
    factory: *mut AMFFactory,
}

// 安全性: AMF のコンポーネントはスレッドセーフではない (Sync は実装しない)。
// Send のみ実装することで、生成スレッドから使用スレッドへの所有権移動を許可する。
// 同時アクセスはコンパイラが Sync の欠如によって防止する。
unsafe impl Send for AmfLibrary {}

impl AmfLibrary {
    /// AMF ランタイムライブラリをロードする
    pub fn load() -> Result<Self, Error> {
        let lib = dl::DynLib::open(Path::new(AMF_DLL_NAME)).map_err(|e| {
            Error::new_custom(
                "AmfLibrary::load",
                &format!("failed to load {AMF_DLL_NAME}: {e}"),
            )
        })?;

        let amf_init: AMFInit_Fn = unsafe { lib.get(b"AMFInit") }.map_err(|e| {
            Error::new_custom("AmfLibrary::load", &format!("failed to find AMFInit: {e}"))
        })?;

        let mut factory: *mut AMFFactory = ptr::null_mut();
        let result = unsafe { amf_init(AMF_FULL_VERSION, &mut factory) };
        Error::check(result, "AMFInit")?;

        if factory.is_null() {
            return Err(Error::new_custom(
                "AmfLibrary::load",
                "AMFInit returned null factory",
            ));
        }

        Ok(Self { _lib: lib, factory })
    }

    /// AMF ランタイムのバージョンを問い合わせる
    pub fn query_version(&self) -> Result<(u16, u16, u16, u16), Error> {
        let amf_query_version: AMFQueryVersion_Fn = unsafe { self._lib.get(b"AMFQueryVersion") }
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

    /// AMFFactory への生ポインタを返す
    pub(crate) fn factory(&self) -> *mut AMFFactory {
        self.factory
    }

    /// AMFContext を作成する
    pub(crate) fn create_context(&self) -> Result<*mut AMFContext, Error> {
        let mut context: *mut AMFContext = ptr::null_mut();
        let result = unsafe {
            let vtbl = &*(*self.factory).pVtbl;
            vtbl.CreateContext.unwrap()(self.factory, &mut context)
        };
        Error::check(result, "AMFFactory::CreateContext")?;

        if context.is_null() {
            return Err(Error::new_custom(
                "AmfLibrary::create_context",
                "CreateContext returned null",
            ));
        }

        Ok(context)
    }

    /// コンテキストに Vulkan デバイスを初期化する (Linux)
    ///
    /// AMFContext から QueryInterface で AMFContext1 を取得し、
    /// InitVulkan(NULL) でデフォルトの Vulkan デバイスを初期化する。
    pub(crate) fn init_vulkan(context: *mut AMFContext) -> Result<(), Error> {
        // AMFContext1 を QueryInterface で取得する
        let mut context1_ptr: *mut c_void = ptr::null_mut();
        let result = unsafe {
            let vtbl = &*(*context).pVtbl;
            vtbl.QueryInterface.unwrap()(context, &IID_AMF_CONTEXT1, &mut context1_ptr)
        };
        Error::check(result, "AMFContext::QueryInterface(AMFContext1)")?;

        if context1_ptr.is_null() {
            return Err(Error::new_custom(
                "AmfLibrary::init_vulkan",
                "QueryInterface returned null for AMFContext1",
            ));
        }

        let context1 = context1_ptr as *mut AMFContext1;

        // Vulkan デバイスを初期化する (NULL = デフォルトデバイス)
        let result = unsafe {
            let vtbl = &*(*context1).pVtbl;
            vtbl.InitVulkan.unwrap()(context1, ptr::null_mut())
        };

        // Context1 の参照を解放する
        unsafe {
            let vtbl = &*(*context1).pVtbl;
            vtbl.Release.unwrap()(context1);
        }

        Error::check(result, "AMFContext1::InitVulkan")
    }
}
