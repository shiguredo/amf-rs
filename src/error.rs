//! AMF エラー型

use std::borrow::Cow;

use crate::sys::AMF_RESULT;

/// AMF 操作のエラー型
#[derive(Debug)]
pub struct Error {
    /// AMF_RESULT のステータスコード
    status: Option<AMF_RESULT>,
    /// エラーが発生した関数名
    function: Cow<'static, str>,
    /// エラーメッセージ
    message: String,
}

/// AMF_RESULT コードに対応する名前とメッセージの対応表
const STATUS_TABLE: &[(AMF_RESULT, &str, &str)] = &[
    (AMF_RESULT::AMF_OK, "AMF_OK", "Success"),
    (AMF_RESULT::AMF_FAIL, "AMF_FAIL", "General failure"),
    (
        AMF_RESULT::AMF_UNEXPECTED,
        "AMF_UNEXPECTED",
        "Unexpected error",
    ),
    (
        AMF_RESULT::AMF_ACCESS_DENIED,
        "AMF_ACCESS_DENIED",
        "Access denied",
    ),
    (
        AMF_RESULT::AMF_INVALID_ARG,
        "AMF_INVALID_ARG",
        "Invalid argument",
    ),
    (
        AMF_RESULT::AMF_OUT_OF_RANGE,
        "AMF_OUT_OF_RANGE",
        "Out of range",
    ),
    (
        AMF_RESULT::AMF_OUT_OF_MEMORY,
        "AMF_OUT_OF_MEMORY",
        "Out of memory",
    ),
    (
        AMF_RESULT::AMF_INVALID_POINTER,
        "AMF_INVALID_POINTER",
        "Invalid pointer",
    ),
    (
        AMF_RESULT::AMF_NO_INTERFACE,
        "AMF_NO_INTERFACE",
        "No interface",
    ),
    (
        AMF_RESULT::AMF_NOT_IMPLEMENTED,
        "AMF_NOT_IMPLEMENTED",
        "Not implemented",
    ),
    (
        AMF_RESULT::AMF_NOT_SUPPORTED,
        "AMF_NOT_SUPPORTED",
        "Not supported",
    ),
    (AMF_RESULT::AMF_NOT_FOUND, "AMF_NOT_FOUND", "Not found"),
    (
        AMF_RESULT::AMF_ALREADY_INITIALIZED,
        "AMF_ALREADY_INITIALIZED",
        "Already initialized",
    ),
    (
        AMF_RESULT::AMF_NOT_INITIALIZED,
        "AMF_NOT_INITIALIZED",
        "Not initialized",
    ),
    (
        AMF_RESULT::AMF_INVALID_FORMAT,
        "AMF_INVALID_FORMAT",
        "Invalid data format",
    ),
    (
        AMF_RESULT::AMF_WRONG_STATE,
        "AMF_WRONG_STATE",
        "Wrong state",
    ),
    (
        AMF_RESULT::AMF_FILE_NOT_OPEN,
        "AMF_FILE_NOT_OPEN",
        "Cannot open file",
    ),
    (AMF_RESULT::AMF_NO_DEVICE, "AMF_NO_DEVICE", "No device"),
    (
        AMF_RESULT::AMF_DIRECTX_FAILED,
        "AMF_DIRECTX_FAILED",
        "DirectX failed",
    ),
    (
        AMF_RESULT::AMF_OPENCL_FAILED,
        "AMF_OPENCL_FAILED",
        "OpenCL failed",
    ),
    (AMF_RESULT::AMF_GLX_FAILED, "AMF_GLX_FAILED", "GLX failed"),
    (AMF_RESULT::AMF_XV_FAILED, "AMF_XV_FAILED", "XV failed"),
    (
        AMF_RESULT::AMF_ALSA_FAILED,
        "AMF_ALSA_FAILED",
        "ALSA failed",
    ),
    (AMF_RESULT::AMF_EOF, "AMF_EOF", "End of file"),
    (AMF_RESULT::AMF_REPEAT, "AMF_REPEAT", "Repeat"),
    (
        AMF_RESULT::AMF_INPUT_FULL,
        "AMF_INPUT_FULL",
        "Input queue is full",
    ),
    (
        AMF_RESULT::AMF_RESOLUTION_CHANGED,
        "AMF_RESOLUTION_CHANGED",
        "Resolution changed",
    ),
    (
        AMF_RESULT::AMF_RESOLUTION_UPDATED,
        "AMF_RESOLUTION_UPDATED",
        "Resolution updated",
    ),
    (
        AMF_RESULT::AMF_INVALID_DATA_TYPE,
        "AMF_INVALID_DATA_TYPE",
        "Invalid data type",
    ),
    (
        AMF_RESULT::AMF_INVALID_RESOLUTION,
        "AMF_INVALID_RESOLUTION",
        "Invalid resolution",
    ),
    (
        AMF_RESULT::AMF_CODEC_NOT_SUPPORTED,
        "AMF_CODEC_NOT_SUPPORTED",
        "Codec not supported",
    ),
    (
        AMF_RESULT::AMF_SURFACE_FORMAT_NOT_SUPPORTED,
        "AMF_SURFACE_FORMAT_NOT_SUPPORTED",
        "Surface format not supported",
    ),
    (
        AMF_RESULT::AMF_DECODER_NOT_PRESENT,
        "AMF_DECODER_NOT_PRESENT",
        "Decoder not present",
    ),
    (
        AMF_RESULT::AMF_DECODER_SURFACE_ALLOCATION_FAILED,
        "AMF_DECODER_SURFACE_ALLOCATION_FAILED",
        "Decoder surface allocation failed",
    ),
    (
        AMF_RESULT::AMF_DECODER_NO_FREE_SURFACES,
        "AMF_DECODER_NO_FREE_SURFACES",
        "Decoder no free surfaces",
    ),
    (
        AMF_RESULT::AMF_ENCODER_NOT_PRESENT,
        "AMF_ENCODER_NOT_PRESENT",
        "Encoder not present",
    ),
    (
        AMF_RESULT::AMF_NEED_MORE_INPUT,
        "AMF_NEED_MORE_INPUT",
        "Need more input",
    ),
    (
        AMF_RESULT::AMF_VULKAN_FAILED,
        "AMF_VULKAN_FAILED",
        "Vulkan failed",
    ),
];

impl Error {
    /// カスタムエラーを作成する
    pub fn new_custom(function: &'static str, message: &str) -> Self {
        Self {
            status: None,
            function: Cow::Borrowed(function),
            message: message.to_string(),
        }
    }

    /// AMF_RESULT からエラーを作成する
    pub fn from_amf(status: AMF_RESULT, function: impl Into<Cow<'static, str>>) -> Self {
        let (name, msg) = STATUS_TABLE
            .iter()
            .find(|(s, _, _)| *s == status)
            .map(|(_, name, msg)| (*name, *msg))
            .unwrap_or(("UNKNOWN", "Unknown error"));

        Self {
            status: Some(status),
            function: function.into(),
            message: format!("{msg} ({name})"),
        }
    }

    /// AMF_RESULT をチェックし、AMF_OK でなければエラーを返す
    pub fn check(status: AMF_RESULT, function: impl Into<Cow<'static, str>>) -> Result<(), Error> {
        if status == AMF_RESULT::AMF_OK {
            Ok(())
        } else {
            Err(Error::from_amf(status, function))
        }
    }

    /// AMF_RESULT のステータスコードを返す
    pub fn status(&self) -> Option<AMF_RESULT> {
        self.status
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(status) => write!(
                f,
                "{}() failed[status={:?}]: {}",
                self.function, status, self.message
            ),
            None => write!(f, "{}(): {}", self.function, self.message),
        }
    }
}

impl std::error::Error for Error {}

/// vtable の関数ポインタが存在することを検証し、欠けている場合はエラーを返す
///
/// AMF ランタイムの vtable エントリが null の場合に panic ではなく
/// `Result::Err` で失敗させるためのヘルパー。
pub(crate) fn require_vtbl_fn<F>(f: Option<F>, name: &str) -> Result<F, Error> {
    f.ok_or_else(|| Error::new_custom("vtable", &format!("missing vtable entry: {name}")))
}

/// AMF が返す `amf_int32` を `usize` に安全に変換する
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::from_amf(AMF_RESULT::AMF_FAIL, "TestFunction");
        let msg = err.to_string();
        assert!(msg.contains("TestFunction"));
        assert!(msg.contains("AMF_FAIL"));
    }

    #[test]
    fn test_check_ok() {
        assert!(Error::check(AMF_RESULT::AMF_OK, "test").is_ok());
    }

    #[test]
    fn test_check_error() {
        assert!(Error::check(AMF_RESULT::AMF_FAIL, "test").is_err());
    }

    #[test]
    fn test_custom_error() {
        let err = Error::new_custom("func", "custom message");
        assert!(err.status().is_none());
        assert!(err.to_string().contains("custom message"));
    }
}
