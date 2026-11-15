//! `AMFComponent::QueryOutput` の結果分類
//!
//! エンコーダーとデコーダーのワーカーで共通に使う、`QueryOutput` の戻り値の
//! 分類を提供する。

use crate::sys::AMF_RESULT;

/// `QueryOutput` のポーリング結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrainStatus {
    /// 現在取得できる出力がない
    Drained,
    /// ドレインが完了した
    Eof,
}

/// `QueryOutput` の結果の扱い
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryOutputKind {
    /// 出力データが取得できた
    Output,
    /// 出力データがない
    NoOutput,
    /// ドレインが完了した
    Eof,
    /// ワーカーを終了させるエラー
    TerminalError,
}

/// `QueryOutput` の結果を分類する
///
/// `has_data` は `QueryOutput` の `ppData` が非 null だったかどうか。
/// AMF は `AMF_OK` でもデータが null の場合があり、AMF デコーダーは
/// `AMF_REPEAT` でも有効なデータを返す場合があるため、`AMF_OK` と
/// `AMF_REPEAT` はデータの有無で出力の有無を決める。
///
/// `AMF_RESOLUTION_CHANGED` と `AMF_RESOLUTION_UPDATED` は解像度変化の通知であり、
/// 出力でも終端エラーでもないため、進捗なしとして扱う。
///
/// 分類は AMF API Reference v1.5.2 の `AMFComponent::QueryOutput()` に基づく。
/// AMF のバージョン更新で仕様が変わる可能性がある。
pub(crate) fn classify_query_output(result: AMF_RESULT, has_data: bool) -> QueryOutputKind {
    match result {
        AMF_RESULT::AMF_OK | AMF_RESULT::AMF_REPEAT => {
            if has_data {
                QueryOutputKind::Output
            } else {
                QueryOutputKind::NoOutput
            }
        }
        AMF_RESULT::AMF_EOF => QueryOutputKind::Eof,
        AMF_RESULT::AMF_RESOLUTION_CHANGED | AMF_RESULT::AMF_RESOLUTION_UPDATED => {
            QueryOutputKind::NoOutput
        }
        _ => QueryOutputKind::TerminalError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AMF_OK でデータがある場合は出力として扱う
    #[test]
    fn test_amf_ok_with_data_is_output() {
        assert_eq!(
            classify_query_output(AMF_RESULT::AMF_OK, true),
            QueryOutputKind::Output,
            "AMF_OK でデータがある場合は Output であること"
        );
    }

    /// AMF_OK でデータがない場合は出力なしとして扱う
    #[test]
    fn test_amf_ok_without_data_is_no_output() {
        assert_eq!(
            classify_query_output(AMF_RESULT::AMF_OK, false),
            QueryOutputKind::NoOutput,
            "AMF_OK でデータがない場合は NoOutput であること"
        );
    }

    /// AMF_REPEAT でデータがある場合は出力として扱う
    #[test]
    fn test_amf_repeat_with_data_is_output() {
        assert_eq!(
            classify_query_output(AMF_RESULT::AMF_REPEAT, true),
            QueryOutputKind::Output,
            "AMF_REPEAT でデータがある場合は Output であること"
        );
    }

    /// AMF_REPEAT でデータがない場合は出力なしとして扱う
    #[test]
    fn test_amf_repeat_without_data_is_no_output() {
        assert_eq!(
            classify_query_output(AMF_RESULT::AMF_REPEAT, false),
            QueryOutputKind::NoOutput,
            "AMF_REPEAT でデータがない場合は NoOutput であること"
        );
    }

    /// AMF_EOF はドレイン完了として扱う
    #[test]
    fn test_amf_eof_is_eof() {
        assert_eq!(
            classify_query_output(AMF_RESULT::AMF_EOF, false),
            QueryOutputKind::Eof,
            "AMF_EOF は Eof であること"
        );
    }

    /// 解像度変化は終端エラーにせず、進捗なしとして扱う
    #[test]
    fn test_resolution_change_is_not_terminal() {
        for result in [
            AMF_RESULT::AMF_RESOLUTION_CHANGED,
            AMF_RESULT::AMF_RESOLUTION_UPDATED,
        ] {
            assert_eq!(
                classify_query_output(result, false),
                QueryOutputKind::NoOutput,
                "{result:?} は NoOutput であること"
            );
        }
    }

    /// 上記以外の結果は終端エラーとして扱う
    #[test]
    fn test_other_results_are_terminal() {
        for result in [
            AMF_RESULT::AMF_FAIL,
            AMF_RESULT::AMF_NO_DEVICE,
            AMF_RESULT::AMF_INVALID_ARG,
        ] {
            assert_eq!(
                classify_query_output(result, false),
                QueryOutputKind::TerminalError,
                "{result:?} は TerminalError であること"
            );
        }
    }
}
