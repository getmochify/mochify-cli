use crate::api::{MochifyClient, PdfParams, ProcessParams};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SquishInput {
    #[schemars(
        description = "Absolute path to the input image file on the user's local macOS filesystem (e.g. /Users/username/Desktop/photo.jpg). Ask the user for the path if you don't know it."
    )]
    pub file_path: String,

    #[schemars(description = "Output format: jpg, png, webp, avif, or jxl")]
    #[serde(rename = "type")]
    pub format: Option<String>,

    #[schemars(description = "Target width in pixels")]
    pub width: Option<u32>,

    #[schemars(description = "Target height in pixels")]
    pub height: Option<u32>,

    #[schemars(description = "Crop image to exact dimensions")]
    pub crop: Option<bool>,

    #[schemars(description = "Rotation in degrees: 0, 90, 180, or 270")]
    pub rotation: Option<u32>,

    #[schemars(
        description = "Absolute output directory path on the user's local macOS filesystem. Defaults to same directory as input file."
    )]
    pub output_dir: Option<String>,

    #[schemars(
        description = "Optional base name for the output file (without extension). When set, the output is saved as <name>.<format> instead of deriving the name from the input file."
    )]
    pub output_name: Option<String>,

    #[schemars(
        description = "Apply clarity — midtone contrast enhancement that makes images look crisper and more detailed without affecting overall exposure"
    )]
    pub clarity: Option<bool>,

    #[schemars(
        description = "Remove the image background (AI foreground isolation). Pair with `background` to composite the cut-out subject onto a solid colour; omit `background` for a transparent result on PNG/WebP/AVIF/JXL."
    )]
    pub remove_background: Option<bool>,

    #[schemars(
        description = "Background colour when removing background. Omit for transparent (default for PNG/WebP/AVIF/JXL). Use \"white\", \"black\", or a hex value like \"#ff0000\". JPEG always composites (default white)."
    )]
    pub background: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PdfInput {
    #[schemars(
        description = "Absolute path to the input PDF file on the user's local macOS filesystem (e.g. /Users/username/Desktop/document.pdf). Ask the user for the path if you don't know it."
    )]
    pub file_path: String,

    #[schemars(
        description = "Operation: \"split\" to split each page into its own PDF, or \"rasterize\" to render pages to images. Defaults to \"rasterize\"."
    )]
    pub op: Option<String>,

    #[schemars(
        description = "Output image format for rasterize: png, jpg, or webp. Defaults to png. Ignored when op is \"split\"."
    )]
    #[serde(rename = "type")]
    pub format: Option<String>,

    #[schemars(
        description = "Rasterize resolution in DPI (e.g. 150 for screen, 300 for print). Defaults to 150. Ignored when op is \"split\"."
    )]
    pub dpi: Option<u32>,

    #[schemars(
        description = "Output quality 1-100 for lossy rasterize formats (jpg/webp). Ignored for split and PNG."
    )]
    pub quality: Option<u32>,

    #[schemars(
        description = "Absolute output directory path on the user's local macOS filesystem. Defaults to same directory as input file."
    )]
    pub output_dir: Option<String>,
}

#[derive(Clone)]
pub struct MochifyMcp {
    pub api_key: Option<String>,
    tool_router: ToolRouter<Self>,
}

impl MochifyMcp {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl MochifyMcp {
    #[tool(
        description = "Process an image using the mochify.app API. Supports format conversion (jpg/png/webp/avif/jxl), resizing, cropping, and rotation."
    )]
    async fn squish(&self, Parameters(input): Parameters<SquishInput>) -> String {
        let path = PathBuf::from(&input.file_path);

        let out_dir = match input.output_dir {
            Some(ref d) => PathBuf::from(d),
            None => path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
        };

        let client = MochifyClient::new(self.api_key.clone());
        let params = ProcessParams {
            format: input.format,
            width: input.width,
            height: input.height,
            crop: input.crop,
            rotation: input.rotation,
            out_name_suffix: None,
            output_name: input.output_name,
            clarity: input.clarity,
            remove_background: input.remove_background,
            background: input.background,
        };

        match client.squish(&path, &params, &out_dir).await {
            Ok((out_path, _meta)) => {
                let usage_note = match client.get_usage().await {
                    Ok(u) => format!(" ({} requests remaining today)", u.remaining),
                    Err(_) => String::new(),
                };
                format!("Saved to {}{}", out_path.display(), usage_note)
            }
            Err(e) => format!("Error: {e:#}"),
        }
    }

    #[tool(
        description = "Process a PDF using the mochify.app API. Either split each page into its own PDF (op=\"split\"), or rasterize pages to images — PNG/JPEG/WebP — at a chosen DPI (op=\"rasterize\"). The result is saved as a .zip in the output directory."
    )]
    async fn pdf(&self, Parameters(input): Parameters<PdfInput>) -> String {
        let path = PathBuf::from(&input.file_path);

        let out_dir = match input.output_dir {
            Some(ref d) => PathBuf::from(d),
            None => path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
        };

        let op = input
            .op
            .unwrap_or_else(|| "rasterize".to_string())
            .to_lowercase();
        if op != "split" && op != "rasterize" {
            return format!("Error: unknown op '{op}'. Use 'split' or 'rasterize'.");
        }

        let params = if op == "rasterize" {
            PdfParams {
                op,
                format: Some(input.format.unwrap_or_else(|| "png".to_string())),
                dpi: Some(input.dpi.unwrap_or(150)),
                quality: input.quality,
            }
        } else {
            PdfParams {
                op,
                format: None,
                dpi: None,
                quality: None,
            }
        };

        let client = MochifyClient::new(self.api_key.clone());
        match client.pdf(&path, &params, &out_dir).await {
            Ok(out_path) => {
                let usage_note = match client.get_usage().await {
                    Ok(u) => format!(" ({} requests remaining today)", u.remaining),
                    Err(_) => String::new(),
                };
                format!("Saved to {}{}", out_path.display(), usage_note)
            }
            Err(e) => format!("Error: {e:#}"),
        }
    }
}

#[tool_handler]
impl ServerHandler for MochifyMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "You have access to the mochify image processing API via two tools. \
                 Both read files directly from the user's local filesystem — you do NOT \
                 need to read the file yourself, and you can access local files. \
                 Use the squish tool for any image task (compression, format conversion, \
                 resizing, cropping, rotation, background removal). \
                 Use the pdf tool for PDF tasks: splitting a PDF into per-page PDFs, or \
                 rasterizing pages to PNG/JPEG/WebP images at a given DPI. \
                 If the user has not provided a file path, ask them for the full path."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
