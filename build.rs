//! AMF SDK のヘッダーファイルから bindgen で FFI バインディングを生成するビルドスクリプト。
//! AMF ランタイム自体のコンパイルやリンクは行わない。

use std::{
    path::{Path, PathBuf},
    process::Command,
};

// 依存ライブラリの名前
const LIB_NAME: &str = "AMF";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build");
    let src_dir = out_build_dir.join(LIB_NAME);
    let include_dir = src_dir.join("amf").join("public").join("include");
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");
    if out_build_dir.exists() {
        std::fs::remove_dir_all(&out_build_dir).expect("failed to remove build directory");
    }
    std::fs::create_dir(&out_build_dir).expect("failed to create build directory");

    // Cargo.toml から依存ライブラリの Git URL とバージョンタグを取得する
    let (git_url, version) = get_git_url_and_version();

    // 各種メタデータを書き込む
    std::fs::write(
        &output_metadata_path,
        format!(
            concat!(
                "pub const BUILD_METADATA_REPOSITORY: &str={:?};\n",
                "pub const BUILD_METADATA_VERSION: &str={:?};\n",
            ),
            git_url, version
        ),
    )
    .expect("failed to write metadata file");

    if std::env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドでは git clone ができないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な定義だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        write_docs_rs_bindings(&output_bindings_path);
        let output_properties_path = out_dir.join("properties.rs");
        std::fs::write(
            &output_properties_path,
            "// docs.rs: プロパティ定数は省略\n",
        )
        .expect("failed to write properties.rs");
        return;
    }

    // 依存ライブラリのリポジトリを取得する
    git_clone_external_lib(&out_build_dir, &git_url, &version);

    // ラッパーヘッダーを生成する
    let wrapper_path = out_dir.join("wrapper.h");
    std::fs::write(
        &wrapper_path,
        [
            "#include <core/Platform.h>",
            "#include <core/Result.h>",
            "#include <core/Interface.h>",
            "#include <core/Variant.h>",
            "#include <core/PropertyStorage.h>",
            "#include <core/PropertyStorageEx.h>",
            "#include <core/Data.h>",
            "#include <core/Plane.h>",
            "#include <core/Surface.h>",
            "#include <core/Buffer.h>",
            "#include <core/Context.h>",
            "#include <core/Factory.h>",
            "#include <components/Component.h>",
            "#include <components/VideoEncoderVCE.h>",
            "#include <components/VideoEncoderHEVC.h>",
            "#include <components/VideoEncoderAV1.h>",
            "#include <components/VideoDecoderUVD.h>",
        ]
        .join("\n"),
    )
    .expect("failed to write wrapper.h");

    // バインディングを生成する
    bindgen::Builder::default()
        .header(wrapper_path.to_str().expect("invalid wrapper path"))
        .clang_arg(format!(
            "-I{}",
            include_dir.to_str().expect("invalid include path")
        ))
        // AMF ヘッダーは macOS でもパース可能 (非 Windows/非 Linux パスで
        // 呼び出し規約マクロが空に定義される)
        // AMF 関連の型と定数のみ生成する (システムヘッダー由来の定義を除外)
        .allowlist_type("AMF.*")
        .allowlist_type("amf_.*")
        .allowlist_var("AMF.*")
        .allowlist_var("amf_.*")
        .allowlist_function("AMF.*")
        .allowlist_function("amf_.*")
        // 列挙型を Rust enum として生成する
        .rustified_enum("AMF_RESULT")
        // AMF が将来追加した値も保持できるよう、AMF が返す列挙型は newtype にする
        .newtype_enum("AMF_SURFACE_FORMAT")
        .newtype_enum("AMF_MEMORY_TYPE")
        .newtype_enum("AMF_DATA_TYPE")
        .rustified_enum("AMF_PLANE_TYPE")
        .newtype_enum("AMF_VARIANT_TYPE")
        .rustified_enum("AMF_DX_VERSION")
        // レート制御メソッドは H.264 と HEVC / AV1 で値の並びが異なるため、
        // 設定値の指定にはコーデック別の enum を使う
        .rustified_enum("AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM")
        .rustified_enum("AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM")
        .rustified_enum("AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM")
        // dl.rs の get<T> で使用するため Option でラップしない型を除外する
        .blocklist_type("AMFInit_Fn")
        .blocklist_type("AMFQueryVersion_Fn")
        // 手動で定義する定数を除外する (型や値が bindgen の出力と異なるため)
        .blocklist_item("AMF_SECOND")
        .blocklist_item("AMF_MILLISECOND")
        .blocklist_item("AMF_MICROSECOND")
        .blocklist_item("AMF_DLL_NAME")
        .blocklist_item("AMF_DLL_NAMEA")
        .derive_default(true)
        .derive_debug(true)
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(&output_bindings_path)
        .expect("failed to write bindings");

    // ワイド文字列マクロ (#define FOO L"bar") から Rust 定数を生成する
    let components_dir = include_dir.join("components");
    let output_properties_path = out_dir.join("properties.rs");
    generate_wide_string_constants(
        &components_dir,
        &[
            "VideoEncoderVCE.h",
            "VideoEncoderHEVC.h",
            "VideoEncoderAV1.h",
            "VideoDecoderUVD.h",
        ],
        &output_properties_path,
    );
}

// docs.rs ビルド用のダミーバインディングを出力する
fn write_docs_rs_bindings(path: &Path) {
    std::fs::write(
        path,
        r#"
pub type amf_int8 = i8;
pub type amf_int16 = i16;
pub type amf_int32 = i32;
pub type amf_int64 = i64;
pub type amf_uint8 = u8;
pub type amf_uint16 = u16;
pub type amf_uint32 = u32;
pub type amf_uint64 = u64;
pub type amf_size = usize;
pub type amf_bool = u8;
pub type amf_long = std::ffi::c_long;
pub type amf_int = std::ffi::c_int;
pub type amf_uint = std::ffi::c_uint;
pub type amf_float = f32;
pub type amf_double = f64;
pub type amf_handle = *mut std::ffi::c_void;
pub type amf_pts = amf_int64;
pub type amf_flags = amf_uint32;

pub type wchar_t = std::os::raw::c_int;

pub const AMF_VERSION_MAJOR: u32 = 1;
pub const AMF_VERSION_MINOR: u32 = 5;
pub const AMF_VERSION_RELEASE: u32 = 0;
pub const AMF_VERSION_BUILD_NUM: u32 = 0;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AMF_RESULT { AMF_OK = 0, AMF_FAIL = 1 }
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AMF_SURFACE_FORMAT(pub amf_uint);
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AMF_MEMORY_TYPE(pub amf_uint);
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AMF_DATA_TYPE(pub amf_uint);
impl AMF_DATA_TYPE {
    pub const AMF_DATA_BUFFER: AMF_DATA_TYPE = AMF_DATA_TYPE(0);
    pub const AMF_DATA_SURFACE: AMF_DATA_TYPE = AMF_DATA_TYPE(1);
    pub const AMF_DATA_AUDIO_BUFFER: AMF_DATA_TYPE = AMF_DATA_TYPE(2);
    pub const AMF_DATA_USER: AMF_DATA_TYPE = AMF_DATA_TYPE(1000);
}
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AMF_PLANE_TYPE { AMF_PLANE_UNKNOWN = 0, AMF_PLANE_PACKED = 1, AMF_PLANE_Y = 2, AMF_PLANE_UV = 3, AMF_PLANE_U = 4, AMF_PLANE_V = 5 }
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AMF_VARIANT_TYPE(pub amf_uint);
pub type AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM = amf_uint;
pub const AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_IDR: AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM = 0;
pub const AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_I: AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM = 1;
pub const AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_P: AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM = 2;
pub const AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_B: AMF_VIDEO_ENCODER_OUTPUT_DATA_TYPE_ENUM = 3;
pub type AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM = amf_uint;
pub const AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_IDR: AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM = 0;
pub const AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_I: AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM = 1;
pub const AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM_AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_P: AMF_VIDEO_ENCODER_HEVC_OUTPUT_DATA_TYPE_ENUM = 2;
pub type AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM = amf_uint;
pub const AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_KEY: AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM = 0;
pub const AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_INTRA_ONLY: AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM = 1;
pub const AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_INTER: AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM = 2;
pub const AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_SWITCH: AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM = 3;
pub const AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM_AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_SHOW_EXISTING: AMF_VIDEO_ENCODER_AV1_OUTPUT_FRAME_TYPE_ENUM = 4;
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM {
    AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_UNKNOWN = -1,
    AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_CONSTANT_QP = 0,
    AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_CBR = 1,
    AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR = 2,
    AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR = 3,
    AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_QUALITY_VBR = 4,
    AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR = 5,
    AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR = 6,
}
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM {
    AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_UNKNOWN = -1,
    AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_CONSTANT_QP = 0,
    AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR = 1,
    AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR = 2,
    AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_CBR = 3,
    AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_QUALITY_VBR = 4,
    AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR = 5,
    AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR = 6,
}
#[repr(i32)]
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM {
    AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_UNKNOWN = -1,
    AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_CONSTANT_QP = 0,
    AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_LATENCY_CONSTRAINED_VBR = 1,
    AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_PEAK_CONSTRAINED_VBR = 2,
    AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_CBR = 3,
    AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_QUALITY_VBR = 4,
    AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_HIGH_QUALITY_VBR = 5,
    AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_HIGH_QUALITY_CBR = 6,
}
pub type AMF_VIDEO_DECODER_MODE_ENUM = amf_uint;
pub const AMF_VIDEO_DECODER_MODE_ENUM_AMF_VIDEO_DECODER_MODE_REGULAR: AMF_VIDEO_DECODER_MODE_ENUM = 0;
pub const AMF_VIDEO_DECODER_MODE_ENUM_AMF_VIDEO_DECODER_MODE_COMPLIANT: AMF_VIDEO_DECODER_MODE_ENUM = 1;
pub const AMF_VIDEO_DECODER_MODE_ENUM_AMF_VIDEO_DECODER_MODE_LOW_LATENCY: AMF_VIDEO_DECODER_MODE_ENUM = 2;
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFRect { pub left: amf_int32, pub top: amf_int32, pub right: amf_int32, pub bottom: amf_int32 }
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFSize { pub width: amf_int32, pub height: amf_int32 }
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFRate { pub num: amf_uint32, pub den: amf_uint32 }
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFRatio { pub num: amf_uint32, pub den: amf_uint32 }
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AMFGuid { pub data1: amf_uint32, pub data2: amf_uint16, pub data3: amf_uint16, pub data41: amf_uint8, pub data42: amf_uint8, pub data43: amf_uint8, pub data44: amf_uint8, pub data45: amf_uint8, pub data46: amf_uint8, pub data47: amf_uint8, pub data48: amf_uint8 }

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFPoint { pub x: amf_int32, pub y: amf_int32 }
#[repr(C)]
#[derive(Copy, Clone)]
pub struct AMFColor { pub __bindgen_anon_1: AMFColor__bindgen_ty_1 }
#[repr(C)]
#[derive(Copy, Clone)]
pub union AMFColor__bindgen_ty_1 { pub rgba: amf_uint32 }
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFFloatSize { pub width: amf_float, pub height: amf_float }
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFFloatPoint2D { pub x: amf_float, pub y: amf_float }
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFFloatPoint3D { pub x: amf_float, pub y: amf_float, pub z: amf_float }
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AMFFloatVector4D { pub x: amf_float, pub y: amf_float, pub z: amf_float, pub w: amf_float }
#[repr(C)]
pub struct AMFInterface { pub pVtbl: *const std::ffi::c_void }

#[repr(C)]
#[derive(Clone, Copy)]
pub union AMFVariantStruct__bindgen_ty_1 {
    pub boolValue: amf_bool,
    pub int64Value: amf_int64,
    pub doubleValue: amf_double,
    pub floatValue: amf_float,
    pub stringValue: *mut std::os::raw::c_char,
    pub wstringValue: *mut wchar_t,
    pub pInterface: *mut AMFInterface,
    pub rectValue: AMFRect,
    pub sizeValue: AMFSize,
    pub pointValue: AMFPoint,
    pub rateValue: AMFRate,
    pub ratioValue: AMFRatio,
    pub colorValue: AMFColor,
    pub floatSizeValue: AMFFloatSize,
    pub floatPoint2DValue: AMFFloatPoint2D,
    pub floatPoint3DValue: AMFFloatPoint3D,
    pub floatVector4DValue: AMFFloatVector4D,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AMFVariantStruct {
    pub type_: AMF_VARIANT_TYPE,
    pub __bindgen_anon_1: AMFVariantStruct__bindgen_ty_1,
}

pub type AMF_ACCELERATION_TYPE = std::ffi::c_int;

#[repr(C)]
pub struct AMFCaps { pub pVtbl: *const AMFCapsVtbl }
#[repr(C)]
#[derive(Default, Copy, Clone)]
pub struct AMFCapsVtbl {
    pub Release: Option<unsafe extern "C" fn(*mut AMFCaps) -> amf_long>,
    pub GetAccelerationType: Option<unsafe extern "C" fn(*mut AMFCaps) -> AMF_ACCELERATION_TYPE>,
    pub GetInputCaps: Option<unsafe extern "C" fn(*mut AMFCaps, *mut *mut AMFIOCaps) -> AMF_RESULT>,
    pub GetOutputCaps: Option<unsafe extern "C" fn(*mut AMFCaps, *mut *mut AMFIOCaps) -> AMF_RESULT>,
    pub GetPropertyCount: Option<unsafe extern "C" fn(*mut AMFCaps) -> amf_size>,
    pub GetPropertyAt: Option<unsafe extern "C" fn(*mut AMFCaps, amf_size, *mut wchar_t, amf_size, *mut AMFVariantStruct) -> AMF_RESULT>,
    pub GetProperty: Option<unsafe extern "C" fn(*mut AMFCaps, *const wchar_t, *mut AMFVariantStruct) -> AMF_RESULT>,
}
#[repr(C)]
pub struct AMFIOCaps { pub pVtbl: *const AMFIOCapsVtbl }
#[repr(C)]
#[derive(Default, Copy, Clone)]
pub struct AMFIOCapsVtbl {
    pub Release: Option<unsafe extern "C" fn(*mut AMFIOCaps) -> amf_long>,
    pub GetWidthRange: Option<unsafe extern "C" fn(*mut AMFIOCaps, *mut amf_int32, *mut amf_int32)>,
    pub GetHeightRange: Option<unsafe extern "C" fn(*mut AMFIOCaps, *mut amf_int32, *mut amf_int32)>,
    pub GetVertAlign: Option<unsafe extern "C" fn(*mut AMFIOCaps) -> amf_int32>,
    pub GetNumOfFormats: Option<unsafe extern "C" fn(*mut AMFIOCaps) -> amf_int32>,
    pub GetFormatAt: Option<unsafe extern "C" fn(*mut AMFIOCaps, amf_int32, *mut AMF_SURFACE_FORMAT, *mut amf_bool) -> AMF_RESULT>,
    pub GetNumOfMemoryTypes: Option<unsafe extern "C" fn(*mut AMFIOCaps) -> amf_int32>,
    pub GetMemoryTypeAt: Option<unsafe extern "C" fn(*mut AMFIOCaps, amf_int32, *mut AMF_MEMORY_TYPE, *mut amf_bool) -> AMF_RESULT>,
    pub IsInterlacedSupported: Option<unsafe extern "C" fn(*mut AMFIOCaps) -> amf_bool>,
}

#[repr(C)]
pub struct AMFFactory { pub pVtbl: *const std::ffi::c_void }
#[repr(C)]
pub struct AMFContext { pub pVtbl: *const std::ffi::c_void }
#[repr(C)]
pub struct AMFContext1 { pub pVtbl: *const std::ffi::c_void }
#[repr(C)]
pub struct AMFPropertyStorage { pub pVtbl: *const std::ffi::c_void }
#[repr(C)]
pub struct AMFComponent { pub pVtbl: *const std::ffi::c_void }
#[repr(C)]
pub struct AMFSurface { pub pVtbl: *const std::ffi::c_void }
#[repr(C)]
pub struct AMFBuffer { pub pVtbl: *const std::ffi::c_void }
#[repr(C)]
pub struct AMFData { pub pVtbl: *const std::ffi::c_void }
#[repr(C)]
pub struct AMFPlane { pub pVtbl: *const std::ffi::c_void }
"#,
    )
    .expect("failed to write docs.rs bindings");
}

// 外部ライブラリのリポジトリを git clone する
fn git_clone_external_lib(build_dir: &Path, git_url: &str, version: &str) {
    let status = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--quiet")
        .arg("--branch")
        .arg(version)
        .arg(git_url)
        .current_dir(build_dir)
        .status()
        .expect("failed to execute git");
    if !status.success() {
        panic!("failed to clone {LIB_NAME} repository: {status}");
    }
}

// Cargo.toml から依存ライブラリの Git URL とバージョンタグを取得する
fn get_git_url_and_version() -> (String, String) {
    let cargo_toml =
        shiguredo_toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    let deps = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get("amf"))
        .unwrap_or_else(|| {
            panic!("Cargo.toml does not contain [package.metadata.external-dependencies.amf]")
        });
    let git_url = deps
        .get("url")
        .and_then(|s| s.as_str())
        .unwrap_or_else(|| panic!("missing 'url' in external-dependencies.amf"));
    let version = deps
        .get("version")
        .and_then(|s| s.as_str())
        .unwrap_or_else(|| panic!("missing 'version' in external-dependencies.amf"));
    // git clone には .git サフィックスが必要
    let clone_url = if git_url.ends_with(".git") {
        git_url.to_string()
    } else {
        format!("{git_url}.git")
    };
    (clone_url, version.to_string())
}

/// ヘッダーファイルからワイド文字列マクロ (#define FOO L"bar") を抽出して
/// Rust の &str 定数として出力する
fn generate_wide_string_constants(
    components_dir: &Path,
    header_files: &[&str],
    output_path: &Path,
) {
    let mut output = String::new();
    output.push_str("// AMF ヘッダーのワイド文字列マクロから自動生成された定数\n");
    output.push_str("// このファイルは build.rs によって自動生成される\n\n");

    // 重複マクロ名を検出するためのセット
    // (プラットフォーム別 #ifdef で同名マクロが異なる値で定義されるケースがある)
    let mut seen = std::collections::HashSet::new();

    for header_file in header_files {
        let header_path = components_dir.join(header_file);
        let content = std::fs::read_to_string(&header_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", header_path.display()));

        output.push_str(&format!("// {header_file}\n"));

        for line in content.lines() {
            let line = line.trim();
            // #define MACRO_NAME L"string_value" にマッチ
            if let Some(rest) = line.strip_prefix("#define ")
                && let Some((name, after_name)) = split_first_whitespace(rest)
                && let Some(value) = extract_wide_string(after_name.trim())
                && seen.insert(name.to_string())
            {
                output.push_str(&format!("pub const {name}: &str = {value:?};\n"));
            }
        }
        output.push('\n');
    }

    std::fs::write(output_path, output).expect("failed to write properties.rs");
}

/// 最初の空白文字で文字列を分割する
fn split_first_whitespace(s: &str) -> Option<(&str, &str)> {
    let pos = s.find(|c: char| c.is_whitespace())?;
    Some((s[..pos].trim(), s[pos..].trim()))
}

/// L"..." 形式のワイド文字列リテラルから文字列を抽出する
fn extract_wide_string(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.starts_with("L\"") {
        return None;
    }
    // L" の後から次の " までを取得する
    let inner = &s[2..];
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}
