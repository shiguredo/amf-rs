//! AMF FFI 定義
//!
//! AMD AMF (Advanced Media Framework) の C インターフェースのバインディング。
//! bindgen でヘッダーから自動生成された定義と、
//! bindgen で生成できない補助定義 (プロパティ名定数、ヘルパー関数等) を含む。

// ビルド時に生成されるメタデータ
mod metadata {
    #![expect(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/metadata.rs"));
}
pub use metadata::*;

// bindgen で AMF ヘッダーから自動生成されたバインディング
mod bindings {
    // 生成コードは C の命名規則をそのまま持つため、Rust の命名規則の lint に
    // 適合しない。生成コードだけに lint の抑制を閉じ込め、手書きコードには
    // 抑制を及ぼさない。
    #![expect(non_camel_case_types)]
    #![expect(non_snake_case)]
    #![expect(non_upper_case_globals)]
    #![expect(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
pub use bindings::*;

/// AMF ヘッダーのマクロから生成した文字列定数
///
/// AMF のコンポーネント ID とプロパティ名を表す。定数値は `&str` であり、
/// AMF へ渡す際にワイド文字列へ変換する。
pub mod str {
    // 定数名は AMF ヘッダーのマクロ名をそのまま使うため、大文字だけの
    // 命名規則には従わない。
    #![expect(non_upper_case_globals)]
    #![expect(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/properties.rs"));
}

// ---------------------------------------------------------------------------
// bindgen で生成できない定義を手動で追加する
// ---------------------------------------------------------------------------

/// 100 ナノ秒単位の 1 秒
pub const AMF_SECOND: amf_pts = 10_000_000;

/// AMF_FULL_VERSION (ビットシフト式は bindgen で生成できない)
pub const AMF_FULL_VERSION: amf_uint64 = (AMF_VERSION_MAJOR as amf_uint64) << 48
    | (AMF_VERSION_MINOR as amf_uint64) << 32
    | (AMF_VERSION_RELEASE as amf_uint64) << 16
    | AMF_VERSION_BUILD_NUM as amf_uint64;

/// Linux x86_64 の AMD GPU ドライバーが提供する AMF ランタイムライブラリ名
pub const AMF_DLL_NAME: &str = "libamfrt64.so.1";

// ---------------------------------------------------------------------------
// DLL エントリーポイント関数型
// ライブラリから取得できたことを前提に呼び出す関数ポインタの型のため、
// Option でラップしない
// ---------------------------------------------------------------------------
/// `AMFInit` の関数ポインタ型
///
/// `version` には要求する AMF のバージョンを、`pp_factory` には
/// `AMFFactory` のポインタを書き込む先のポインタを指定する。
pub type AmfInitFn =
    unsafe extern "C" fn(version: amf_uint64, pp_factory: *mut *mut AMFFactory) -> AMF_RESULT;

/// `AMFQueryVersion` の関数ポインタ型
///
/// `p_version` には AMF のバージョンを受け取るポインタを指定する。
pub type AmfQueryVersionFn = unsafe extern "C" fn(p_version: *mut amf_uint64) -> AMF_RESULT;

// ---------------------------------------------------------------------------
// AMFContext1 の IID
// AMF_DECLARE_IID マクロで定義されるが、bindgen は定数として生成しない
// ---------------------------------------------------------------------------

/// AMFContext1 の IID (IID_AMFContext1: {d9e9f868-6220-44c6-a22f-7cd6dac68646})
pub const IID_AMF_CONTEXT1: AMFGuid = AMFGuid {
    data1: 0xd9e9f868,
    data2: 0x6220,
    data3: 0x44c6,
    data41: 0xa2,
    data42: 0x2f,
    data43: 0x7c,
    data44: 0xd6,
    data45: 0xda,
    data46: 0xc6,
    data47: 0x86,
    data48: 0x46,
};

// ---------------------------------------------------------------------------
// AMFVariantStruct ヘルパーメソッド
//
// フィールド名は bindgen の生成結果に依存する。
// AMF SDK の Variant.h では AMFVariantStruct の共用体は
// 匿名共用体または名前付きフィールドで定義される。
// ---------------------------------------------------------------------------

/// bindgen が生成する匿名共用体の型別名
pub type AMFVariantValue = AMFVariantStruct__bindgen_ty_1;

impl AMFVariantStruct {
    /// 空の Variant を作成する
    pub fn empty() -> Self {
        Self {
            type_: AMF_VARIANT_TYPE::AMF_VARIANT_EMPTY,
            __bindgen_anon_1: AMFVariantValue { int64Value: 0 },
        }
    }

    /// Int64 型の Variant を作成する
    pub fn from_int64(val: amf_int64) -> Self {
        Self {
            type_: AMF_VARIANT_TYPE::AMF_VARIANT_INT64,
            __bindgen_anon_1: AMFVariantValue { int64Value: val },
        }
    }

    /// Bool 型の Variant を作成する
    pub fn from_bool(val: bool) -> Self {
        Self {
            type_: AMF_VARIANT_TYPE::AMF_VARIANT_BOOL,
            __bindgen_anon_1: AMFVariantValue {
                boolValue: val as amf_bool,
            },
        }
    }

    /// Double 型の Variant を作成する
    pub fn from_double(val: amf_double) -> Self {
        Self {
            type_: AMF_VARIANT_TYPE::AMF_VARIANT_DOUBLE,
            __bindgen_anon_1: AMFVariantValue { doubleValue: val },
        }
    }

    /// Rate 型の Variant を作成する
    pub fn from_rate(num: amf_uint32, den: amf_uint32) -> Self {
        Self {
            type_: AMF_VARIANT_TYPE::AMF_VARIANT_RATE,
            __bindgen_anon_1: AMFVariantValue {
                rateValue: AMFRate { num, den },
            },
        }
    }

    /// Size 型の Variant を作成する
    pub fn from_size(width: amf_int32, height: amf_int32) -> Self {
        Self {
            type_: AMF_VARIANT_TYPE::AMF_VARIANT_SIZE,
            __bindgen_anon_1: AMFVariantValue {
                sizeValue: AMFSize { width, height },
            },
        }
    }

    /// Ratio 型の Variant を作成する
    pub fn from_ratio(num: amf_uint32, den: amf_uint32) -> Self {
        Self {
            type_: AMF_VARIANT_TYPE::AMF_VARIANT_RATIO,
            __bindgen_anon_1: AMFVariantValue {
                ratioValue: AMFRatio { num, den },
            },
        }
    }
}

// ---------------------------------------------------------------------------
// ワイド文字列ヘルパー
//
// Linux x86_64 の wchar_t は 32 ビットの符号付き整数であり、AMF の
// ワイド文字列 (amf_int32 の配列) とそのまま相互変換できる。
// ---------------------------------------------------------------------------

/// ヌル終端の AMF ワイド文字列を Rust 文字列に変換する
///
/// Linux x86_64 の `wchar_t` (UTF-32) を前提とする。
pub fn from_wstring(buf: &[amf_int32]) -> String {
    buf.iter()
        .take_while(|&&c| c != 0)
        .filter_map(|&c| char::from_u32(c as u32))
        .collect()
}

/// Rust 文字列をヌル終端の AMF ワイド文字列に変換する
///
/// Linux x86_64 の `wchar_t` (UTF-32) を前提とする。
pub fn to_wstring(s: &str) -> Vec<amf_int32> {
    s.chars()
        .map(|c| c as amf_int32)
        .chain(std::iter::once(0))
        .collect()
}
