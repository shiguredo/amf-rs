//! AMF FFI 定義
//!
//! AMD AMF (Advanced Media Framework) の C インターフェースのバインディング。
//! bindgen でヘッダーから自動生成された定義と、
//! bindgen で生成できない補助定義 (プロパティ名定数、ヘルパー関数等) を含む。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(dead_code)]
#![allow(clippy::all)]

// ビルド時に生成されるメタデータ
include!(concat!(env!("OUT_DIR"), "/metadata.rs"));

// bindgen で AMF ヘッダーから自動生成されたバインディング
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// ヘッダーのワイド文字列マクロから自動生成されたプロパティ名定数
include!(concat!(env!("OUT_DIR"), "/properties.rs"));

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

/// Linux x86_64 での AMF ランタイムライブラリ名
pub const AMF_DLL_NAME: &str = "libamfrt64.so.1";

// ---------------------------------------------------------------------------
// DLL エントリーポイント関数型
// dl.rs の get<T> で直接使用するため Option でラップしない
// ---------------------------------------------------------------------------
pub type AMFInit_Fn =
    unsafe extern "C" fn(version: amf_uint64, ppFactory: *mut *mut AMFFactory) -> AMF_RESULT;
pub type AMFQueryVersion_Fn = unsafe extern "C" fn(pVersion: *mut amf_uint64) -> AMF_RESULT;

// ---------------------------------------------------------------------------
// AMFContext1 の IID
// AMF_DECLARE_IID マクロで定義されるが、bindgen では名前が異なる場合がある
// ---------------------------------------------------------------------------

/// IID_AMFContext1: {d9e9f868-6220-44c6-a22f-7cd6dac68646}
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
// ビルドエラーが発生した場合、OUT_DIR/bindings.rs の
// AMFVariantStruct 定義を確認して修正すること。
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
// ワイド文字列ヘルパー (Linux: wchar_t = i32 = UTF-32)
// ---------------------------------------------------------------------------

/// Rust 文字列をヌル終端の UTF-32 (Linux wchar_t) 配列に変換する
pub fn to_wstring(s: &str) -> Vec<amf_int32> {
    s.chars()
        .map(|c| c as amf_int32)
        .chain(std::iter::once(0))
        .collect()
}
