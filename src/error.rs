//! AMF エラー型
//!
//! 利用者の入力不正、オブジェクトの状態不正、AMF の呼び出し失敗、実行環境の
//! 失敗、ライブラリと AMF の前提違反を型で区別する。

use std::borrow::Cow;
use std::error::Error as StdError;
use std::sync::mpsc;

use crate::sys::AMF_RESULT;

/// AMF 操作のエラー
///
/// 利用者は variant を match して失敗の種類を判別し、分類型から詳細を取得する。
#[derive(Debug)]
pub enum Error {
    /// 利用者が渡した値が API の契約を満たさない
    InvalidArgument(InvalidArgumentError),
    /// オブジェクトの状態が操作に適していない
    InvalidState(InvalidStateError),
    /// AMF の呼び出しがエラー status を返した
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

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Environment(error) => Some(&error.source),
            Self::Internal(error) => error.source(),
            Self::InvalidArgument(_) | Self::InvalidState(_) | Self::Amf(_) => None,
        }
    }
}

/// 利用者が渡した値が API の契約を満たさないエラー
///
/// 値を修正して呼び出し直すことで解消できる。
#[derive(Debug)]
pub struct InvalidArgumentError {
    /// 不合格となった値と、満たす必要がある契約
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
///
/// 呼び出し順を直すことで解消できる。
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

/// AMF の呼び出しがエラー status を返したエラー
#[derive(Debug)]
pub struct AmfError {
    /// AMF が返した status
    status: AMF_RESULT,
    /// エラー status を返した AMF の操作
    operation: Cow<'static, str>,
}

impl AmfError {
    /// AMF の status と操作からエラーを作成する
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

    /// AMF の status を確認し、`AMF_OK` 以外ならエラーを返す
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

    /// AMF が返した status を返す
    pub fn status(&self) -> AMF_RESULT {
        self.status
    }

    /// エラー status を返した AMF の操作名を返す
    pub fn operation(&self) -> &str {
        &self.operation
    }
}

impl std::fmt::Display for AmfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}() failed: {:?}", self.operation, self.status)
    }
}

/// 実行環境に起因するエラー
///
/// 動的ライブラリのロードやシンボル取得、ワーカースレッドの起動など、
/// ライブラリの状態では解消できない失敗を表す。
#[derive(Debug)]
pub struct EnvironmentError {
    /// 失敗した処理の文脈
    message: Cow<'static, str>,
    /// 原因となった OS のエラー
    source: std::io::Error,
}

impl EnvironmentError {
    /// 文脈メッセージと原因の OS エラーを指定してエラーを作成する
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

/// ライブラリまたは AMF の前提が破られたエラー
///
/// AMF が `AMF_OK` を返したのに null を返した場合や、vtable の関数ポインタが
/// 欠けている場合など、ライブラリの想定外の状態を表す。
#[derive(Debug)]
pub struct InternalError {
    /// 破られた前提
    message: Cow<'static, str>,
    /// 原因となった標準エラー
    source: Option<Box<dyn StdError + Send + Sync>>,
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
        source: impl Into<Box<dyn StdError + Send + Sync>>,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(source.into()),
        }
    }

    /// 原因となった標準エラーを返す
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn StdError + 'static))
    }
}

impl std::fmt::Display for InternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "internal error: {}", self.message)
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
        InternalError::with_source("worker thread terminated", source).into()
    }
}

impl<T> From<mpsc::SendError<T>> for Error {
    fn from(_source: mpsc::SendError<T>) -> Self {
        // 未送信のコマンドは保持せず、チャネルの切断だけをエラーとして扱う。
        InternalError::new("worker thread terminated").into()
    }
}

/// vtable の関数ポインタが存在することを確認し、欠けている場合はエラーを返す
///
/// AMF ランタイムの vtable エントリが null の場合に panic ではなく
/// `Result::Err` で失敗させるためのヘルパー。
pub(crate) fn require_vtbl_fn<F>(f: Option<F>, name: &str) -> Result<F, Error> {
    f.ok_or_else(|| InternalError::new(format!("missing vtable entry: {name}")).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AMF のエラー表示に status 名が 1 回だけ現れることを確認する
    #[test]
    fn test_amf_error_display_outputs_status_once() {
        let error = AmfError::new(AMF_RESULT::AMF_FAIL, "AMFComponent::Init");
        let message = error.to_string();
        assert_eq!(
            message.matches("AMF_FAIL").count(),
            1,
            "status 名が 1 回だけ表示されていない: {message}"
        );
        assert!(
            message.contains("AMFComponent::Init() failed"),
            "操作名が表示されていない: {message}"
        );
    }

    /// AMF_OK がエラーとして構築できないことを確認する
    #[test]
    #[should_panic(expected = "AMF_OK is not an AMF error status")]
    fn test_amf_error_rejects_amf_ok() {
        AmfError::new(AMF_RESULT::AMF_OK, "AMFComponent::Init");
    }

    /// Environment と Internal の source() が内側の標準エラーを返すことを確認する
    #[test]
    fn test_error_source_returns_inner_error() {
        let error: Error = EnvironmentError::new(
            "failed to load library",
            std::io::Error::other("no such file"),
        )
        .into();
        let source = error.source().expect("source が取得できない");
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

        let source = std::ffi::CString::new("null\0byte").expect_err("NulError にならない");
        let error: Error = source.into();
        assert!(
            matches!(error, Error::Internal(_)),
            "Internal ではない: {error}"
        );
        let source = error.source().expect("source が取得できない");
        assert!(
            source.to_string().contains("nul byte"),
            "source の内容が一致しない: {source}"
        );
    }

    /// source を持たない分類の source() が None になることを確認する
    #[test]
    fn test_source_is_none_without_standard_error() {
        let error: Error = InvalidArgumentError::new("invalid value").into();
        assert!(error.source().is_none(), "source が返された: {error}");

        let (tx, rx) = mpsc::channel::<()>();
        drop(rx);
        let send_error = tx.send(()).expect_err("送信が成功した");
        let error: Error = send_error.into();
        assert!(
            matches!(error, Error::Internal(_)),
            "Internal ではない: {error}"
        );
        assert!(error.source().is_none(), "source が返された: {error}");
    }
}
