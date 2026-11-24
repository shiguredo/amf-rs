//! `AMFComponent::QueryOutput` の結果分類
//!
//! ワーカーの `QueryOutput` ポーリングで共通に使う、戻り値と `ppData` の分類、
//! およびポーリング状態を提供する。

use crate::sys::AMF_RESULT;

/// `QueryOutput` のポーリングで得られた状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrainStatus {
    /// 現時点で取得できる出力がない
    NotReady,
    /// ドレインが完了した
    Eof,
}

/// `QueryOutput` の結果の扱い
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryOutputKind {
    /// 出力として扱うデータが取得できた
    Output,
    /// 出力として扱わない結果
    NoOutput,
    /// ドレインが完了した
    Eof,
    /// 継続できない終端エラー
    TerminalError,
}

/// `QueryOutput` の結果を分類する
///
/// `has_data` は `QueryOutput` の `ppData` が非 null だったかどうか。
/// データの有無より先にステータスを見る。`AMF_OK` と `AMF_REPEAT` はデータが
/// 非 null なら出力、null なら出力なしとして扱う。`AMF_REPEAT` でデータを返す
/// 実装があるため、データがある場合は出力として扱う。
///
/// `AMF_RESOLUTION_CHANGED` と `AMF_RESOLUTION_UPDATED` は出力として扱わない。
/// `AMF_EOF` はドレイン完了、それ以外は終端エラーとして扱う。
// 分類は AMF API Reference v1.5.2 §2.6.1 の AMFComponent::QueryOutput() に基づく。
// AMF_OK + null と AMF_REPEAT は同節の戻り値の説明に対応する。
// AMF_RESOLUTION_CHANGED / AMF_RESOLUTION_UPDATED は include/core/Result.h の
// 汎用コードで、QueryOutput の戻り値表には記載がない。
// AMF_REPEAT + 非 null のデータは AMF デコーダーの実測挙動と、AMF サンプル実装が
// ppData を優先して出力として扱うことに合わせる。
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

    /// ステータスとデータの有無から結果が正しく分類されることを確認する
    #[test]
    fn test_classify_query_output() {
        let cases = [
            (
                AMF_RESULT::AMF_OK,
                true,
                QueryOutputKind::Output,
                "AMF_OK でデータがある",
            ),
            (
                AMF_RESULT::AMF_OK,
                false,
                QueryOutputKind::NoOutput,
                "AMF_OK でデータがない",
            ),
            (
                AMF_RESULT::AMF_REPEAT,
                true,
                QueryOutputKind::Output,
                "AMF_REPEAT でデータがある",
            ),
            (
                AMF_RESULT::AMF_REPEAT,
                false,
                QueryOutputKind::NoOutput,
                "AMF_REPEAT でデータがない",
            ),
            (AMF_RESULT::AMF_EOF, false, QueryOutputKind::Eof, "AMF_EOF"),
            (
                AMF_RESULT::AMF_EOF,
                true,
                QueryOutputKind::Eof,
                "AMF_EOF はデータの有無よりステータスを優先する",
            ),
            (
                AMF_RESULT::AMF_RESOLUTION_CHANGED,
                false,
                QueryOutputKind::NoOutput,
                "AMF_RESOLUTION_CHANGED",
            ),
            (
                AMF_RESULT::AMF_RESOLUTION_CHANGED,
                true,
                QueryOutputKind::NoOutput,
                "AMF_RESOLUTION_CHANGED はデータがあっても出力にしない",
            ),
            (
                AMF_RESULT::AMF_RESOLUTION_UPDATED,
                false,
                QueryOutputKind::NoOutput,
                "AMF_RESOLUTION_UPDATED",
            ),
            (
                AMF_RESULT::AMF_RESOLUTION_UPDATED,
                true,
                QueryOutputKind::NoOutput,
                "AMF_RESOLUTION_UPDATED はデータがあっても出力にしない",
            ),
            (
                AMF_RESULT::AMF_FAIL,
                false,
                QueryOutputKind::TerminalError,
                "AMF_FAIL",
            ),
            (
                AMF_RESULT::AMF_FAIL,
                true,
                QueryOutputKind::TerminalError,
                "AMF_FAIL はデータがあっても終端エラー",
            ),
            (
                AMF_RESULT::AMF_NO_DEVICE,
                false,
                QueryOutputKind::TerminalError,
                "AMF_NO_DEVICE",
            ),
            (
                AMF_RESULT::AMF_INVALID_ARG,
                false,
                QueryOutputKind::TerminalError,
                "AMF_INVALID_ARG",
            ),
        ];

        for (result, has_data, expected, description) in cases {
            assert_eq!(
                classify_query_output(result, has_data),
                expected,
                "{description}: result={result:?}, has_data={has_data}"
            );
        }
    }
}
