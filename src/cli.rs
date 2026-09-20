use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "mochify",
    about = "CLI for the mochify.app image processing API"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Input image file(s)
    pub files: Vec<PathBuf>,

    /// Output format: jpg | png | webp | avif | jxl
    #[arg(short = 't', long = "type", value_name = "FORMAT")]
    pub format: Option<String>,

    /// Target width in pixels
    #[arg(short, long, value_name = "N")]
    pub width: Option<u32>,

    /// Target height in pixels
    #[arg(short = 'H', long, value_name = "N")]
    pub height: Option<u32>,

    /// Crop to exact dimensions
    #[arg(long)]
    pub crop: bool,

    /// Rotation in degrees (0, 90, 180, 270)
    #[arg(short, long, value_name = "DEG")]
    pub rotation: Option<u32>,

    /// Output directory [default: same directory as input]
    #[arg(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Base name for the output file (without extension)
    #[arg(short = 'n', long, value_name = "NAME")]
    pub name: Option<String>,

    /// Apply clarity (midtone contrast enhancement)
    #[arg(long)]
    pub clarity: bool,

    /// Remove the image background (AI foreground isolation)
    #[arg(long = "remove-bg")]
    pub remove_bg: bool,

    /// Composite background colour (e.g. "white", "black", "#ff0000").
    /// Pair with --remove-bg; omit for a transparent result on PNG/WebP/AVIF/JXL.
    #[arg(long = "background", value_name = "COLOR")]
    pub background: Option<String>,

    /// Preserve EXIF/metadata (GPS, timestamps, device info).
    /// Metadata is stripped by default; pass this to keep it.
    #[arg(long = "keep-metadata")]
    pub keep_metadata: bool,

    /// Saliency-guided quality: detailed subjects keep more, flat areas less
    #[arg(long = "smart-compress")]
    pub smart_compress: bool,

    /// Exposure adjustment, -100 (darkest) to +100 (brightest)
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    pub brightness: Option<i32>,

    /// Progressive encoding plus 4:2:0 chroma subsampling — the smallest file to serve
    #[arg(long = "optimize-for-web", alias = "optimise-for-web")]
    pub optimize_for_web: bool,

    /// Pixel-exact output (jxl, webp, png only). Overrides quality and --smart-compress,
    /// and the result is usually larger than the input
    #[arg(long)]
    pub lossless: bool,

    /// Ultra HDR gain map: "preserve" (the default when the flag is bare) keeps a gain
    /// map the source already has; "generate" also creates one for an SDR source.
    /// Only JPEG output can carry a gain map.
    #[arg(
        long,
        value_name = "MODE",
        num_args = 0..=1,
        default_missing_value = "preserve"
    )]
    pub hdr: Option<String>,

    /// Natural-language prompt — calls /v1/prompt to resolve params
    #[arg(short = 'p', long, value_name = "TEXT")]
    pub prompt: Option<String>,

    /// PDF operation: optimize | extract | rasterize | split for .pdf inputs,
    /// or create to build a PDF from images
    #[arg(long, value_name = "OP")]
    pub op: Option<String>,

    /// Resolution in DPI: rasterize render resolution [default: 150], optimize target
    /// resolution for the images kept inside the PDF, create page sizing
    #[arg(long, value_name = "N")]
    pub dpi: Option<u32>,

    /// Output quality 1–100 [default: auto]. Images: overrides --smart-compress, and
    /// 100 is the best lossy setting, not lossless (see --lossless). PDFs: the op's
    /// output quality
    #[arg(short = 'q', long, value_name = "N")]
    pub quality: Option<u32>,

    /// Cap image width in pixels: extract caps each extracted image, optimize caps the
    /// longest side of each image rewritten into the PDF, create downscales before
    /// embedding. 0 leaves sizes alone.
    #[arg(long = "max-width", value_name = "N")]
    pub max_width: Option<u32>,

    /// Skip images smaller than this on either axis (optimize, extract). 0 takes everything.
    #[arg(long = "min-size", value_name = "N")]
    pub min_size: Option<u32>,

    /// Page size when building a PDF from images: fit | a4 | letter [default: fit]
    #[arg(long, value_name = "SIZE")]
    pub page: Option<String>,

    /// With --op create, produce one single-page PDF per image (returned as a zip)
    /// instead of one combined document
    #[arg(long = "no-combine")]
    pub no_combine: bool,

    /// How many files to process at once. 1 runs them strictly one after another;
    /// raise it on a fast uplink, lower it if the API starts answering "at capacity"
    #[arg(short = 'j', long, value_name = "N", default_value_t = 4)]
    pub jobs: usize,

    /// API key for automation/CI [env: MOCHIFY_API_KEY].
    /// Interactive users can run `mochify auth login` instead.
    #[arg(short = 'k', long, env = "MOCHIFY_API_KEY", value_name = "KEY")]
    pub api_key: Option<String>,

    /// Print raw API responses and response headers (useful when exploring the API directly)
    #[arg(short = 'v', long)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start MCP server on stdio
    Serve,
    /// Show API usage for the current key
    Usage,
    /// Authenticate with Mochify via browser
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
}

#[derive(Subcommand)]
pub enum AuthAction {
    /// Open browser to sign in and save credentials locally
    Login,
    /// Remove saved credentials
    Logout,
    /// Show current authentication status
    Status,
}
