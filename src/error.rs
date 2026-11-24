//! AMF エラー型

use std::borrow::Cow;
use std::sync::mpsc;

use crate::sys::AMF_RESULT;

/// AMF 操作のエラー
///
/// 利用者はバリアントを match して失敗の種類を判別できる。
/// AMF のステータスと操作名は [`AmfError`] から取得する。
#[derive(Debug)]
pub enum Error {
    /// 引数、または引数が指すオブジェクトが操作の前提を満たさない
    InvalidArgument(InvalidArgumentError),
    /// オブジェクトの状態が操作に適していない
    InvalidState(InvalidStateError),
    /// AMF の呼び出しがエラーステータスを返した
    Amf(AmfError),
    /// 実行環境に起因する失敗
    Environment(EnvironmentError),
    /// ライブラリまたは AMF の前提が破られた
    Internal(InternalError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument(error) => error.fmt(f),
            Self::InvalidState(error) => error.fmt(f),
            Self::Amf(error) => error.fmt(f),
            Self::Environment(error) => error.fmt(f),
            Self::Internal(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Environment(error) => Some(&error.source),
            Self::Internal(error) => std::error::Error::source(error),
            Self::InvalidArgument(_) | Self::InvalidState(_) | Self::Amf(_) => None,
        }
    }
}

/// 引数、または引数が指すオブジェクトが操作の前提を満たさないエラー
///
/// 値を差し替えるか設定を変えることで解消できる。
#[derive(Debug)]
pub struct InvalidArgumentError {
    /// 不合格の理由
    message: Cow<'static, str>,
}

impl InvalidArgumentError {
    /// 文脈メッセージを指定してエラーを作成する
    pub(crate) fn new(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for InvalidArgumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid argument: {}", self.message)
    }
}

/// オブジェクトの状態が操作に適していないエラー
#[derive(Debug)]
pub struct InvalidStateError {
    /// 操作を拒否した理由
    message: Cow<'static, str>,
}

impl InvalidStateError {
    /// 文脈メッセージを指定してエラーを作成する
    pub(crate) fn new(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for InvalidStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid state: {}", self.message)
    }
}

/// AMF の呼び出しがエラーステータスを返したエラー
#[derive(Debug)]
pub struct AmfError {
    status: AMF_RESULT,
    operation: Cow<'static, str>,
}

impl AmfError {
    /// AMF のステータスと操作からエラーを作成する
    ///
    /// # Panics
    ///
    /// `status` が `AMF_OK` の場合にパニックする。`AMF_OK` は成功を表し、
    /// エラーとして扱えないためである。
    pub(crate) fn new(status: AMF_RESULT, operation: impl Into<Cow<'static, str>>) -> Self {
        assert_ne!(
            status,
            AMF_RESULT::AMF_OK,
            "AMF_OK is not an AMF error status"
        );
        Self {
            status,
            operation: operation.into(),
        }
    }

    /// AMF のステータスを確認し、`AMF_OK` 以外ならエラーを返す
    pub(crate) fn check(
        status: AMF_RESULT,
        operation: impl Into<Cow<'static, str>>,
    ) -> Result<(), Self> {
        if status == AMF_RESULT::AMF_OK {
            Ok(())
        } else {
            Err(Self::new(status, operation))
        }
    }

    /// AMF が返したステータスを返す
    pub fn status(&self) -> AMF_RESULT {
        self.status
    }

    /// エラーステータスを返した AMF の操作名を返す
    pub fn operation(&self) -> &str {
        &self.operation
    }
}

impl std::fmt::Display for AmfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {:?}", self.operation, self.status)
    }
}

/// 実行環境に起因するエラー
///
/// 動的ライブラリのロードやシンボル取得、ワーカースレッドの起動など、
/// ライブラリの状態では解消できない失敗を表す。
/// 原因となった I/O エラーは `source()` で取得する（`std::error::Error` のメソッド）。
/// この分類を `Error` として保持している場合も、`Error::source()` で原因を取得できる。
/// `Display` は文脈だけを表示する。
#[derive(Debug)]
pub struct EnvironmentError {
    /// 失敗した処理の文脈
    message: Cow<'static, str>,
    /// 原因となった I/O エラー
    source: std::io::Error,
}

impl EnvironmentError {
    /// 文脈メッセージと原因の I/O エラーを指定してエラーを作成する
    pub(crate) fn new(message: impl Into<Cow<'static, str>>, source: std::io::Error) -> Self {
        Self {
            message: message.into(),
            source,
        }
    }
}

impl std::fmt::Display for EnvironmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "environment error: {}", self.message)
    }
}

impl std::error::Error for EnvironmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// ライブラリまたは AMF の前提が破られたエラー
///
/// AMF が `AMF_OK` を返したのに null を返した場合や、vtable の関数ポインタが
/// 欠けている場合など、ライブラリの想定外の状態を表す。
/// 原因となる標準エラーがある場合は `source()` で取得する（`std::error::Error` のメソッド）。
/// この分類を `Error` として保持している場合も、`Error::source()` で原因を取得できる。
/// `Display` は文脈だけを表示する。
#[derive(Debug)]
pub struct InternalError {
    /// 破られた前提
    message: Cow<'static, str>,
    /// 原因となった標準エラー
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl InternalError {
    /// 文脈メッセージだけを指定してエラーを作成する
    pub(crate) fn new(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// 文脈メッセージと原因の標準エラーを指定してエラーを作成する
    pub(crate) fn with_source(
        message: impl Into<Cow<'static, str>>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(source.into()),
        }
    }
}

impl std::fmt::Display for InternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "internal error: {}", self.message)
    }
}

impl std::error::Error for InternalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl From<InvalidArgumentError> for Error {
    fn from(error: InvalidArgumentError) -> Self {
        Self::InvalidArgument(error)
    }
}

impl From<InvalidStateError> for Error {
    fn from(error: InvalidStateError) -> Self {
        Self::InvalidState(error)
    }
}

impl From<AmfError> for Error {
    fn from(error: AmfError) -> Self {
        Self::Amf(error)
    }
}

impl From<EnvironmentError> for Error {
    fn from(error: EnvironmentError) -> Self {
        Self::Environment(error)
    }
}

impl From<InternalError> for Error {
    fn from(error: InternalError) -> Self {
        Self::Internal(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        EnvironmentError::new("I/O operation failed", source).into()
    }
}

impl From<std::ffi::NulError> for Error {
    fn from(source: std::ffi::NulError) -> Self {
        InternalError::with_source("string contains a null byte", source).into()
    }
}

impl From<std::num::TryFromIntError> for Error {
    fn from(source: std::num::TryFromIntError) -> Self {
        InternalError::with_source("integer conversion failed", source).into()
    }
}

impl From<mpsc::RecvError> for Error {
    fn from(source: mpsc::RecvError) -> Self {
        InternalError::with_source("worker channel closed", source).into()
    }
}

impl<T> From<mpsc::SendError<T>> for Error {
    fn from(_source: mpsc::SendError<T>) -> Self {
        // 未送信のコマンドは保持せず、チャネルの切断だけをエラーとして扱う。
        InternalError::new("worker channel closed").into()
    }
}

/// vtable の関数ポインタを取得する
///
/// `f` が `Some` の場合はその値を返し、`None` の場合は [`Error::Internal`] を返す。
pub(crate) fn require_vtbl_fn<F>(f: Option<F>, name: &str) -> Result<F, Error> {
    f.ok_or_else(|| InternalError::new(format!("missing vtable entry: {name}")).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Error がワーカースレッドを跨いで渡せることを型で確認する
    #[test]
    fn test_error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Error>();
    }

    /// AMF のエラー表示にステータス名が 1 回だけ現れることを確認する
    #[test]
    fn test_amf_error_display_outputs_status_once() {
        let error = AmfError::new(AMF_RESULT::AMF_FAIL, "AMFComponent::Init");
        let message = error.to_string();
        assert_eq!(
            message.matches("AMF_FAIL").count(),
            1,
            "ステータス名が 1 回だけ表示されていない: {message}"
        );
        assert!(
            message.contains("AMFComponent::Init failed"),
            "操作名が表示されていない: {message}"
        );
    }

    /// AMF_OK がエラーとして構築できないことを確認する
    #[test]
    #[should_panic(expected = "AMF_OK is not an AMF error status")]
    fn test_amf_error_rejects_amf_ok() {
        AmfError::new(AMF_RESULT::AMF_OK, "AMFComponent::Init");
    }

    /// AMF のステータスの確認が AMF_OK を成功、それ以外を失敗として扱うことを確認する
    #[test]
    fn test_amf_error_check() {
        assert!(AmfError::check(AMF_RESULT::AMF_OK, "AMFComponent::Init").is_ok());

        let error = AmfError::check(AMF_RESULT::AMF_FAIL, "AMFComponent::Init")
            .expect_err("AMF_FAIL が成功として扱われた");
        assert_eq!(
            error.status(),
            AMF_RESULT::AMF_FAIL,
            "ステータスが一致しない"
        );
        assert_eq!(
            error.operation(),
            "AMFComponent::Init",
            "operation が一致しない"
        );
    }

    /// Environment と Internal の source() が内側の標準エラーを返すことを確認する
    #[test]
    fn test_error_source_returns_inner_error() {
        let environment = EnvironmentError::new(
            "failed to load library",
            std::io::Error::other("no such file"),
        );
        let source = std::error::Error::source(&environment)
            .expect("EnvironmentError の source が取得できない");
        assert!(
            source.to_string().contains("no such file"),
            "EnvironmentError の source の内容が一致しない: {source}"
        );

        let error: Error = environment.into();
        let source = std::error::Error::source(&error).expect("source が取得できない");
        assert!(
            source.to_string().contains("no such file"),
            "source の内容が一致しない: {source}"
        );
        assert!(
            error
                .to_string()
                .contains("environment error: failed to load library"),
            "文脈メッセージが表示されていない: {error}"
        );

        let nul = std::ffi::CString::new("null\0byte").expect_err("NulError にならない");
        let internal = InternalError::with_source("string contains a null byte", nul);
        assert!(
            std::error::Error::source(&internal).is_some(),
            "InternalError の source が取得できない"
        );

        let error: Error = internal.into();
        assert!(
            matches!(&error, Error::Internal(_)),
            "Internal ではない: {error}"
        );
        let source = std::error::Error::source(&error).expect("source が取得できない");
        assert!(
            source.to_string().contains("nul byte"),
            "source の内容が一致しない: {source}"
        );

        let error: Error = InternalError::new("worker channel closed").into();
        assert!(
            std::error::Error::source(&error).is_none(),
            "source が返された: {error}"
        );
    }

    /// source を持たない分類の source() が None になることを確認する
    #[test]
    fn test_source_is_none_without_standard_error() {
        let error: Error = InvalidArgumentError::new("invalid value").into();
        assert!(
            std::error::Error::source(&error).is_none(),
            "source が返された: {error}"
        );

        let (tx, rx) = mpsc::channel::<()>();
        drop(rx);
        let send_error = tx.send(()).expect_err("送信が成功した");
        let error: Error = send_error.into();
        assert!(
            matches!(&error, Error::Internal(_)),
            "Internal ではない: {error}"
        );
        assert!(
            std::error::Error::source(&error).is_none(),
            "source が返された: {error}"
        );
    }
}
