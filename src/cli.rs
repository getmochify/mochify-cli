use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "mochify", about = "CLI for the mochify.xyz image processing API")]
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

    /// Natural-language prompt — calls /v1/prompt to resolve params
    #[arg(short = 'p', long, value_name = "TEXT")]
    pub prompt: Option<String>,

    /// API key [env: MOCHIFY_API_KEY]
    #[arg(short = 'k', long, env = "MOCHIFY_API_KEY", value_name = "KEY")]
    pub api_key: Option<String>,
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
