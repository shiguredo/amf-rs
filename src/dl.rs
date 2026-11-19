//! Linux の dlopen を利用した動的ライブラリローダー

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::error::{EnvironmentError, Error};

/// 動的ライブラリのハンドル
pub(crate) struct DynLib {
    handle: *mut c_void,
}

impl std::fmt::Debug for DynLib {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynLib").finish_non_exhaustive()
    }
}

unsafe impl Send for DynLib {}

// Linux の dlopen は RTLD_LOCAL が 0 なので、RTLD_LAZY だけを指定する。
const DLOPEN_FLAGS: c_int = 1;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

impl DynLib {
    /// シンボルを取得して指定の型として返す
    ///
    /// # Safety
    ///
    /// - `T` はシンボルが指す関数の正しい型でなければならない
    /// - `T` はポインタサイズの関数ポインタ型であること
    ///   (`assert_eq!` でサイズチェックを行うが、型の正しさは呼び出し側が保証する)
    /// - `transmute_copy` により生ポインタを `T` にビットキャストするため、
    ///   `T` が実際のシンボルの ABI と一致しない場合は未定義動作になる
    ///
    /// # Errors
    ///
    /// シンボル名に NUL バイトが含まれる場合、またはシンボルを取得できない場合に
    /// エラーを返す。
    pub(crate) unsafe fn get<T>(&self, symbol: &[u8]) -> Result<T, Error> {
        assert_eq!(
            std::mem::size_of::<T>(),
            std::mem::size_of::<*mut c_void>(),
            "T must be pointer-sized"
        );
        let c_symbol = CString::new(symbol)?;
        let ptr = unsafe { self.sym(&c_symbol)? };
        Ok(unsafe { std::mem::transmute_copy(&ptr) })
    }

    /// 指定したパスの動的ライブラリを開く
    ///
    /// # Errors
    ///
    /// パスに NUL バイトが含まれる場合、またはライブラリを開けない場合に
    /// エラーを返す。ライブラリを開けない場合の原因は `source()` で取得できる。
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let c_path = CString::new(path.as_os_str().as_bytes())?;
        unsafe {
            dlerror();
            let handle = dlopen(c_path.as_ptr(), DLOPEN_FLAGS);
            if handle.is_null() {
                let err = dlerror();
                let source = if err.is_null() {
                    std::io::Error::other("dlopen failed without a diagnostic message")
                } else {
                    std::io::Error::other(CStr::from_ptr(err).to_string_lossy().into_owned())
                };
                return Err(EnvironmentError::new(
                    format!("failed to load library: {}", path.display()),
                    source,
                )
                .into());
            }
            Ok(DynLib { handle })
        }
    }

    /// シンボルを取得する
    ///
    /// # Errors
    ///
    /// シンボルを取得できない場合にエラーを返す。原因は `source()` で取得できる。
    // POSIX では dlsym がシンボル値として正当に NULL を返すケースがあるが、
    // このクレートでは AMF の関数ポインタのみを取得するため NULL は常にエラーとして扱う。
    pub(crate) unsafe fn sym(&self, symbol: &CString) -> Result<*mut c_void, Error> {
        unsafe {
            dlerror();
            let ptr = dlsym(self.handle, symbol.as_ptr());
            if ptr.is_null() {
                let err = dlerror();
                let source = if err.is_null() {
                    std::io::Error::other("dlsym failed without a diagnostic message")
                } else {
                    std::io::Error::other(CStr::from_ptr(err).to_string_lossy().into_owned())
                };
                return Err(EnvironmentError::new(
                    format!("failed to find symbol: {}", symbol.to_string_lossy()),
                    source,
                )
                .into());
            }
            Ok(ptr)
        }
    }
}

impl Drop for DynLib {
    fn drop(&mut self) {
        unsafe {
            dlclose(self.handle);
        }
    }
}
