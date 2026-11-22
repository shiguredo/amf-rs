//! AMF ランタイムとコーデック対応状況のチェック。
//!
//! AMF ランタイムのロード、バージョン照会、コーデック対応状況の表示、
//! エンコーダー/デコーダーの初期化確認を行う。

use shiguredo_amf::amf::Context;
use shiguredo_amf::{
    AmfLibrary, Av1EncoderConfig, BUILD_VERSION, CodecConfig, CodecInfo, Decoder, DecoderCodec,
    DecoderConfig, Encoder, EncoderConfig, Error, FnDecodeHandler, FnEncodeHandler, FrameFormat,
    H264EncoderConfig, HevcEncoderConfig, RateControlMode, VideoCodecType, VideoProfile,
    supported_codecs,
};

#[derive(Debug)]
enum AppError {
    Args(noargs::Error),
    Amf(Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Args(e) => write!(f, "{e:?}"),
            AppError::Amf(e) => write!(f, "{e}"),
        }
    }
}

impl From<noargs::Error> for AppError {
    fn from(e: noargs::Error) -> Self {
        AppError::Args(e)
    }
}

impl From<Error> for AppError {
    fn from(e: Error) -> Self {
        AppError::Amf(e)
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Args(_) => None,
            AppError::Amf(e) => std::error::Error::source(e),
        }
    }
}

type Result<T> = std::result::Result<T, AppError>;

struct Args {
    width: u32,
    height: u32,
}

fn parse_args() -> Result<Args> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = "Check AMF runtime and codec availability";

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    noargs::HELP_FLAG.take_help(&mut args);

    let width: u32 = noargs::opt("width")
        .doc("Encoder init check width (default: 640)")
        .default("640")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    let height: u32 = noargs::opt("height")
        .doc("Encoder init check height (default: 480)")
        .default("480")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        std::process::exit(0);
    }

    Ok(Args { width, height })
}

fn codec_name(codec: VideoCodecType) -> &'static str {
    match codec {
        VideoCodecType::H264 => "H.264",
        VideoCodecType::Hevc => "H.265",
        VideoCodecType::Av1 => "AV1",
    }
}

fn format_profile(profile: Option<VideoProfile>) -> String {
    match profile {
        Some(profile) => format!("{profile:?}"),
        None => "-".to_string(),
    }
}

fn format_acceleration(acceleration: Option<shiguredo_amf::AccelerationType>) -> String {
    match acceleration {
        Some(acceleration) => format!("{acceleration:?}"),
        None => "-".to_string(),
    }
}

fn format_io_capabilities(caps: Option<&shiguredo_amf::IoCapabilities>) -> String {
    match caps {
        Some(caps) => format!(
            "{}..{}x{}..{} formats={} memory_types={}",
            caps.min_width,
            caps.max_width,
            caps.min_height,
            caps.max_height,
            caps.formats.len(),
            caps.memory_types.len()
        ),
        None => "-".to_string(),
    }
}

/// エラーとその原因を 1 行にまとめる
fn format_error(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = std::error::Error::source(cause);
    }
    message
}

fn check_runtime() -> bool {
    println!("=== AMF Runtime ===");
    println!("Build version: {BUILD_VERSION}");

    print!("Load runtime ... ");
    let lib = AmfLibrary::instance();
    match lib.query_version() {
        Ok(version) => {
            println!("OK");
            println!("Runtime version: {version}");
            true
        }
        Err(e) => {
            println!("NG ({})", format_error(&e));
            false
        }
    }
}

/// コーデック対応状況を表示する
fn print_codec_support(codecs: &Result<Vec<CodecInfo>>) {
    println!();
    println!("=== Codec Support (supported_codecs) ===");
    let codecs = match codecs {
        Ok(codecs) => codecs,
        Err(e) => {
            println!("supported_codecs failed: {}", format_error(&e));
            return;
        }
    };
    for info in codecs {
        let name = codec_name(info.codec);
        println!("{name}:");
        match &info.decoding {
            Some(decoding) => {
                println!(
                    "  decoding: acceleration={} input={} output={} properties={}",
                    format_acceleration(decoding.acceleration),
                    format_io_capabilities(decoding.input.as_ref()),
                    format_io_capabilities(decoding.output.as_ref()),
                    decoding.properties.len()
                );
            }
            None => println!("  decoding: not supported"),
        }
        match &info.encoding {
            Some(encoding) => {
                println!(
                    "  encoding: acceleration={} profile={} level={:?} frame_reordering={:?}",
                    format_acceleration(encoding.acceleration),
                    format_profile(encoding.max_profile),
                    encoding.max_level,
                    encoding.supports_frame_reordering
                );
                println!(
                    "    input={} output={} properties={}",
                    format_io_capabilities(encoding.input.as_ref()),
                    format_io_capabilities(encoding.output.as_ref()),
                    encoding.properties.len()
                );
            }
            None => println!("  encoding: not supported"),
        }
    }
}

fn encoder_config(codec: VideoCodecType, width: u32, height: u32) -> EncoderConfig {
    let codec_config = match codec {
        VideoCodecType::H264 => CodecConfig::H264(H264EncoderConfig::default()),
        VideoCodecType::Hevc => CodecConfig::Hevc(HevcEncoderConfig::default()),
        VideoCodecType::Av1 => CodecConfig::Av1(Av1EncoderConfig::default()),
    };
    let mut config = EncoderConfig::new(
        codec_config,
        width,
        height,
        FrameFormat::Nv12,
        30,
        1,
        RateControlMode::Cbr,
    );
    config.target_kbps = Some(1_000);
    config
}

fn check_encoder_init(context: &Context, codec: VideoCodecType, width: u32, height: u32) -> bool {
    let name = codec_name(codec);
    print!("[encoder] {name} init ({width}x{height}) ... ");
    let config = encoder_config(codec, width, height);
    match Encoder::new(context.clone(), config, FnEncodeHandler::<()>::new(|_| {})) {
        Ok(_encoder) => {
            println!("OK");
            true
        }
        Err(e) => {
            println!("NG ({})", format_error(&e));
            false
        }
    }
}

fn check_decoder_init(context: &Context, codec: VideoCodecType) -> bool {
    let name = codec_name(codec);
    print!("[decoder] {name} init ... ");
    let decoder_codec = match codec {
        VideoCodecType::H264 => DecoderCodec::H264,
        VideoCodecType::Hevc => DecoderCodec::Hevc,
        VideoCodecType::Av1 => DecoderCodec::Av1,
    };
    let config = DecoderConfig {
        codec: decoder_codec,
    };
    match Decoder::new(context.clone(), config, FnDecodeHandler::<()>::new(|_| {})) {
        Ok(_decoder) => {
            println!("OK");
            true
        }
        Err(e) => {
            println!("NG ({})", format_error(&e));
            false
        }
    }
}

/// 対応コーデックのエンコーダー / デコーダーを初期化できるか確認する
fn check_component_init(codecs: &Result<Vec<CodecInfo>>, width: u32, height: u32) -> bool {
    println!();
    println!("=== Component Init ===");

    let context = match AmfLibrary::instance().create_context() {
        Ok(context) => context,
        Err(e) => {
            println!("[context] create ... NG ({})", format_error(&e));
            return false;
        }
    };

    let mut ok = true;
    let codecs = match codecs {
        Ok(codecs) => codecs,
        Err(e) => {
            println!("[context] supported_codecs ... NG ({})", format_error(&e));
            return false;
        }
    };
    for info in codecs {
        if info.encoding.is_some() {
            if !check_encoder_init(&context, info.codec, width, height) {
                ok = false;
            }
        } else {
            println!(
                "[encoder] {} init ... SKIP (not supported)",
                codec_name(info.codec)
            );
        }

        if info.decoding.is_some() {
            if !check_decoder_init(&context, info.codec) {
                ok = false;
            }
        } else {
            println!(
                "[decoder] {} init ... SKIP (not supported)",
                codec_name(info.codec)
            );
        }
    }
    ok
}

fn main() -> Result<()> {
    let args = parse_args()?;

    let mut ok = check_runtime();
    let codecs = supported_codecs().map_err(AppError::Amf);
    print_codec_support(&codecs);

    // ランタイムがロードできていても、エンコーダー/デコーダー初期化が失敗することがある
    if !check_component_init(&codecs, args.width, args.height) {
        ok = false;
    }

    println!();
    if ok {
        println!("All checks passed");
    } else {
        println!("Some checks failed");
        std::process::exit(1);
    }

    Ok(())
}
