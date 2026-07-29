//! AMF ランタイムとコーデック対応状況のチェック。
//!
//! AMF ランタイムのロード、バージョン照会、コーデック対応状況の表示、
//! エンコーダー/デコーダーの初期化確認を行う。

use shiguredo_amf::{
    AmfLibrary, Av1EncoderConfig, BUILD_VERSION, CodecConfig, Decoder, DecoderCodec, DecoderConfig,
    Encoder, EncoderConfig, EncodingProfiles, FnDecodeHandler, FnEncodeHandler, FrameFormat,
    H264EncoderConfig, HevcEncoderConfig, RateControlMode, VideoCodecType, supported_codecs,
};

#[derive(Debug)]
enum Error {
    Args(noargs::Error),
    Amf(shiguredo_amf::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Args(e) => write!(f, "{e:?}"),
            Error::Amf(e) => write!(f, "{e}"),
        }
    }
}

impl From<noargs::Error> for Error {
    fn from(e: noargs::Error) -> Self {
        Error::Args(e)
    }
}

impl From<shiguredo_amf::Error> for Error {
    fn from(e: shiguredo_amf::Error) -> Self {
        Error::Amf(e)
    }
}

type Result<T> = std::result::Result<T, Error>;

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

fn format_profiles(profiles: &EncodingProfiles) -> String {
    match profiles {
        EncodingProfiles::H264(list) => list
            .iter()
            .map(|p| format!("{p:?}"))
            .collect::<Vec<_>>()
            .join(", "),
        EncodingProfiles::Hevc(list) => list
            .iter()
            .map(|p| format!("{p:?}"))
            .collect::<Vec<_>>()
            .join(", "),
        EncodingProfiles::Av1(list) => list
            .iter()
            .map(|p| format!("{p:?}"))
            .collect::<Vec<_>>()
            .join(", "),
        EncodingProfiles::None => "-".to_string(),
    }
}

fn check_runtime() -> bool {
    println!("=== AMF Runtime ===");
    println!("Build version: {BUILD_VERSION}");

    print!("Load runtime ... ");
    let lib = AmfLibrary::instance();
    match lib.query_version() {
        Ok((major, minor, release, build)) => {
            println!("OK");
            println!("Runtime version: {major}.{minor}.{release}.{build}");
            true
        }
        Err(e) => {
            println!("NG ({e})");
            false
        }
    }
}

fn print_codec_support() {
    println!();
    println!("=== Codec Support (supported_codecs) ===");
    for info in supported_codecs() {
        let name = codec_name(info.codec);
        println!("{name}:");
        println!(
            "  decoding: supported={}, hw_accel={}",
            info.decoding.supported, info.decoding.hardware_accelerated
        );
        println!(
            "  encoding: supported={}, hw_accel={}",
            info.encoding.supported, info.encoding.hardware_accelerated
        );
        if info.encoding.supported {
            println!(
                "    frame_reordering={}, multi_pass={}",
                info.encoding.supports_frame_reordering, info.encoding.supports_multi_pass
            );
            println!("    profiles: {}", format_profiles(&info.encoding.profiles));
        }
    }
}

fn encoder_config(codec: VideoCodecType, width: u32, height: u32) -> EncoderConfig {
    let codec_config = match codec {
        VideoCodecType::H264 => CodecConfig::H264(H264EncoderConfig { profile: None }),
        VideoCodecType::Hevc => CodecConfig::Hevc(HevcEncoderConfig { profile: None }),
        VideoCodecType::Av1 => CodecConfig::Av1(Av1EncoderConfig { profile: None }),
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

fn check_encoder_init(codec: VideoCodecType, width: u32, height: u32) -> bool {
    let name = codec_name(codec);
    print!("[encoder] {name} init ({width}x{height}) ... ");
    let config = encoder_config(codec, width, height);
    match Encoder::new(config, FnEncodeHandler::<()>::new(|_| {})) {
        Ok(_encoder) => {
            println!("OK");
            true
        }
        Err(e) => {
            println!("NG ({e})");
            false
        }
    }
}

fn check_decoder_init(codec: VideoCodecType) -> bool {
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
    match Decoder::new(config, FnDecodeHandler::<()>::new(|_| {})) {
        Ok(_decoder) => {
            println!("OK");
            true
        }
        Err(e) => {
            println!("NG ({e})");
            false
        }
    }
}

fn check_component_init(width: u32, height: u32) -> bool {
    println!();
    println!("=== Component Init ===");

    let mut ok = true;
    for info in supported_codecs() {
        if info.encoding.supported {
            if !check_encoder_init(info.codec, width, height) {
                ok = false;
            }
        } else {
            println!(
                "[encoder] {} init ... SKIP (not supported)",
                codec_name(info.codec)
            );
        }

        if info.decoding.supported {
            if !check_decoder_init(info.codec) {
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
    print_codec_support();

    // ランタイムがロードできていても、エンコーダー/デコーダー初期化が失敗することがある
    if !check_component_init(args.width, args.height) {
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
