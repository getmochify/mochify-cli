use crate::api::{MochifyClient, PdfOptions, PdfParams, ProcessParams};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SquishInput {
    #[schemars(
        description = "Absolute path to the input image file on the user's local filesystem \
         (e.g. /Users/me/Desktop/photo.jpg on macOS, /home/me/photo.jpg on Linux, \
         C:\\Users\\me\\Desktop\\photo.jpg on Windows). Ask the user for the path if you \
         don't know it."
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
        description = "Absolute output directory path on the user's local filesystem. Defaults to the same \
         directory as the input file."
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

    #[schemars(
        description = "Strip EXIF/metadata (GPS, timestamps, device identifiers). Defaults to true — metadata is removed. Set false to preserve the original metadata."
    )]
    pub strip_metadata: Option<bool>,

    #[schemars(
        description = "Output quality 1-100. Defaults to automatic selection, which is usually the right choice — set it only when the user asks for a specific quality, or for visibly smaller/higher-quality output. Overrides smart_compress. 100 is the best LOSSY setting, not lossless."
    )]
    pub quality: Option<u32>,

    #[schemars(
        description = "Saliency-guided quality selection: high-detail subjects get more quality, flat areas less. Good default for \"compress this as well as possible without it looking worse\". Ignored when quality is set."
    )]
    pub smart_compress: Option<bool>,

    #[schemars(
        description = "Exposure adjustment from -100 (darkest) to 100 (brightest). 0 is no change. Use for \"brighten this\" / \"it's too dark\"."
    )]
    pub brightness: Option<i32>,

    #[schemars(
        description = "Progressive encoding plus 4:2:0 chroma subsampling — the smallest file for browser delivery. Use for \"optimize this for my website\"."
    )]
    pub optimize_for_web: Option<bool>,

    #[schemars(
        description = "Pixel-exact output. Only jxl, webp and png can hold it — jpg and avif are rejected, so set `type` to one of those three. Overrides quality and smart_compress. Expect the output to be LARGER than the input; a source that is already lossy (JPEG, AVIF, HEIC) comes back as the best lossy encode instead, since nothing can restore what it discarded."
    )]
    pub lossless: Option<bool>,

    #[schemars(
        description = "Ultra HDR gain map handling. \"preserve\" keeps a gain map the source already has (and does nothing to an SDR source); \"generate\" keeps it AND synthesizes one when the source is plain SDR — use \"generate\" whenever the user asks to make something HDR. Only jpg output can carry a gain map, so set `type` to jpg unless the user asked for another format. Ignored alongside remove_background or clarity, which change the base the gain map is measured against."
    )]
    pub hdr: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PdfInput {
    #[schemars(
        description = "Absolute path to the input PDF file on the user's local filesystem \
         (e.g. /Users/me/Desktop/report.pdf on macOS, /home/me/report.pdf on Linux, \
         C:\\Users\\me\\Desktop\\report.pdf on Windows). Ask the user for the path if you \
         don't know it."
    )]
    pub file_path: String,

    #[schemars(
        description = "Operation. \"optimize\" recompresses the images inside the PDF and returns a smaller PDF, leaving text and layout untouched — this is the one for \"compress this PDF\". \"extract\" pulls out the images somebody placed into the document. \"rasterize\" renders every page to an image. \"split\" writes one single-page PDF per page. Defaults to \"rasterize\"."
    )]
    pub op: Option<String>,

    #[schemars(
        description = "Output image format. rasterize: png (default), jpg, webp, avif or jxl. extract: png, jpg, webp, avif, jxl, or \"original\" (default) to copy each embedded image out with no re-encode. Ignored by optimize (images inside a PDF are always JPEG) and split."
    )]
    #[serde(rename = "type")]
    pub format: Option<String>,

    #[schemars(
        description = "Target resolution in DPI. rasterize: the render resolution of each page (150 for screen, 300 for print; default 150). optimize: the resolution to target for the images kept inside the PDF, measured against how large they are drawn on the page (96 for screen and email, 150 general, 300 to keep print quality). Ignored by extract and split."
    )]
    pub dpi: Option<u32>,

    #[schemars(
        description = "Output quality 1-100. Applies to optimize (recompression quality, default 75), extract and rasterize (lossy formats only). Ignored for PNG and for split."
    )]
    pub quality: Option<u32>,

    #[schemars(
        description = "Cap image width in pixels, preserving aspect ratio. extract: caps each extracted image. optimize: caps the longest side of each image rewritten into the PDF. 0 leaves sizes alone. Ignored by rasterize and split."
    )]
    pub max_width: Option<u32>,

    #[schemars(
        description = "Skip images smaller than this on either axis (optimize and extract), which keeps spacers, rules and bullet glyphs out of the result. Defaults to 64; 0 takes everything."
    )]
    pub min_size: Option<u32>,

    #[schemars(
        description = "Absolute output directory path on the user's local filesystem. Defaults to the same \
         directory as the input file."
    )]
    pub output_dir: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PdfCreateInput {
    #[schemars(
        description = "Absolute paths to the input images on the user's local filesystem, in the order \
         they should appear — one page per image. Ask the user for the paths if you don't \
         know them."
    )]
    pub file_paths: Vec<String>,

    #[schemars(
        description = "Page size: \"fit\" (default — each page is exactly its image, no whitespace), \"a4\", or \"letter\"."
    )]
    pub page: Option<String>,

    #[schemars(description = "Quality 1-100 for the JPEG embedded in each page. Defaults to 82.")]
    pub quality: Option<u32>,

    #[schemars(
        description = "Pixels per inch used to size a \"fit\" page, 36-600. Defaults to 96."
    )]
    pub dpi: Option<u32>,

    #[schemars(
        description = "Downscale images wider than this before embedding them. 0 (default) leaves them alone."
    )]
    pub max_width: Option<u32>,

    #[schemars(
        description = "Set false to get one single-page PDF per image, returned as a .zip, instead of one combined document. Defaults to true (one PDF)."
    )]
    pub combine: Option<bool>,

    #[schemars(
        description = "Optional base name for the output file (without extension). Defaults to the first image's name."
    )]
    pub output_name: Option<String>,

    #[schemars(
        description = "Absolute output directory path on the user's local filesystem. Defaults to the \
         directory of the first image."
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
        description = "Process a single image file on the user's local filesystem: format \
         conversion (jpg/png/webp/avif/jxl), resizing, cropping, rotation, background removal, \
         brightness, clarity, quality control (fixed, saliency-guided or lossless), web \
         optimization, and Ultra HDR gain maps. Reads the file and writes the result itself, so \
         do not load the image into the conversation first. Use `pdf` instead for anything that \
         takes a PDF in, and `pdf_create` to build a PDF out of images. Writes a new file beside \
         the input (or in output_dir) and never overwrites the original: a name collision gets a \
         numeric suffix.",
        annotations(
            title = "Compress or convert an image",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
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

        let hdr = match input.hdr.as_deref() {
            Some(mode) => match crate::api::normalize_hdr_mode(mode) {
                Ok(v) => Some(v),
                Err(e) => return format!("Error: {e:#}"),
            },
            None => None,
        };

        if input.lossless == Some(true)
            && let Some(ref fmt) = input.format
        {
            let fmt = fmt.trim().to_lowercase();
            let fmt = if fmt == "jpeg" {
                "jpg".to_string()
            } else {
                fmt
            };
            if !crate::api::LOSSLESS_FORMATS.contains(&fmt.as_str()) {
                return format!(
                    "Error: lossless output is not possible with type {fmt}. Only {} can hold pixel-exact output.",
                    crate::api::LOSSLESS_FORMATS.join(", ")
                );
            }
        }

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
            strip_exif: input.strip_metadata,
            hdr,
            quality: input.quality,
            smart_compress: input.smart_compress,
            brightness: input.brightness,
            optimize_for_web: input.optimize_for_web,
            lossless: input.lossless,
        };

        match client.squish(&path, &params, &out_dir).await {
            Ok((out_path, meta)) => {
                // X-Mochify-HDR describes the bytes that came back, so it is the only
                // honest answer to "did it actually get a gain map?".
                let hdr_note = match (params.hdr.as_ref(), meta.hdr.as_deref()) {
                    (Some(_), Some("true")) => " (HDR gain map preserved from the source)",
                    (Some(_), Some("generated")) => " (HDR gain map generated)",
                    (Some(_), Some("false")) => {
                        " (no HDR gain map in the output — only jpg output can carry one)"
                    }
                    _ => "",
                };
                // "downgraded" means the source was already lossy, so the request could
                // not be honoured literally — say so rather than implying pixel-exactness.
                let lossless_note = match (params.lossless, meta.lossless.as_deref()) {
                    (Some(true), Some("downgraded")) => {
                        " (source was already lossy — encoded at the best lossy setting instead of pixel-exact)"
                    }
                    (Some(true), Some("true")) => " (pixel-exact)",
                    _ => "",
                };
                let usage_note = match client.get_usage().await {
                    Ok(u) => format!(" ({} requests remaining today)", u.remaining),
                    Err(_) => String::new(),
                };
                format!(
                    "Saved to {}{}{}{}",
                    out_path.display(),
                    hdr_note,
                    lossless_note,
                    usage_note
                )
            }
            Err(e) => format!("Error: {e:#}"),
        }
    }

    #[tool(
        description = "Take a PDF file on the user's local filesystem and run one of four \
         operations on it. \"optimize\" recompresses the images inside the PDF and returns a \
         smaller PDF that is still searchable, because text, fonts, vector art and layout are \
         untouched: this is the one for \"compress this PDF\" or \"it is too big to email\". \
         \"extract\" pulls the embedded images out as an archive. \"rasterize\" renders each page \
         to an image (PNG/JPEG/WebP/AVIF/JXL) at a chosen DPI. \"split\" writes one single-page \
         PDF per page. optimize saves a .pdf, the others save a .zip, in the output directory. \
         Use `pdf_create` instead to build a PDF from images, and `squish` for a single image. \
         The four operations here need a paid plan; `pdf_create` does not. Writes a new file and \
         never overwrites the input.",
        annotations(
            title = "Optimize, extract, rasterize or split a PDF",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
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

        let op = input.op.unwrap_or_else(|| "rasterize".to_string());
        if op.trim().eq_ignore_ascii_case("create") {
            return "Error: use the pdf_create tool to build a PDF from images.".to_string();
        }
        let params = match PdfParams::for_op(
            &op,
            PdfOptions {
                format: input.format,
                dpi: input.dpi,
                quality: input.quality,
                max_width: input.max_width,
                min_size: input.min_size,
                ..Default::default()
            },
        ) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e:#}"),
        };

        let client = MochifyClient::new(self.api_key.clone());
        match client.pdf(&path, &params, &out_dir).await {
            Ok((out_path, meta)) => {
                // How much smaller the PDF got is the entire point of optimize, and it
                // only exists in a response header.
                let saved_note = match (params.op.as_str(), meta.saved_pct.as_deref()) {
                    ("optimize", Some("0")) => {
                        " (already well optimized — the original was returned unchanged)"
                            .to_string()
                    }
                    ("optimize", Some(pct)) => format!(" ({pct}% smaller)"),
                    _ => String::new(),
                };
                let usage_note = match client.get_usage().await {
                    Ok(u) => format!(" ({} requests remaining today)", u.remaining),
                    Err(_) => String::new(),
                };
                format!(
                    "Saved to {}{}{}",
                    out_path.display(),
                    saved_note,
                    usage_note
                )
            }
            Err(e) => format!("Error: {e:#}"),
        }
    }

    #[tool(
        description = "Build a PDF out of image files on the user's local filesystem, one page \
         per image, in the order given. Saves a single .pdf, or a .zip of one-page PDFs when \
         combine is false. Use `pdf` instead for anything that takes an existing PDF in. Works \
         on every plan including Free. Writes a new file and never overwrites an input.",
        annotations(
            title = "Build a PDF from images",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn pdf_create(&self, Parameters(input): Parameters<PdfCreateInput>) -> String {
        if input.file_paths.is_empty() {
            return "Error: no images given. Pass the absolute path of each image in page order."
                .to_string();
        }
        let paths: Vec<PathBuf> = input.file_paths.iter().map(PathBuf::from).collect();

        let out_dir = match input.output_dir {
            Some(ref d) => PathBuf::from(d),
            None => paths[0]
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
        };

        let params = match PdfParams::for_op(
            "create",
            PdfOptions {
                dpi: input.dpi,
                quality: input.quality,
                max_width: input.max_width,
                page: input.page,
                combine: input.combine,
                ..Default::default()
            },
        ) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e:#}"),
        };

        let client = MochifyClient::new(self.api_key.clone());
        match client
            .pdf_create(&paths, &params, &out_dir, input.output_name.as_deref())
            .await
        {
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

    // Parity with the hosted server, which has had this since day one. Without it
    // an agent on the local server has no way to answer "how much quota is left?"
    // short of shelling out to `mochify usage`.
    #[tool(
        description = "Check how many operations remain in the current billing period, and on \
         which plan. Takes no parameters. Needs authentication: run `mochify auth login`, or set \
         MOCHIFY_API_KEY for automation. Reports the account's own quota, not the anonymous \
         IP-based allowance.",
        annotations(
            title = "Check remaining quota",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn check_usage(&self) -> String {
        let client = MochifyClient::new(self.api_key.clone());
        match client.get_usage().await {
            Ok(u) => {
                let plan = if u.plan.is_empty() {
                    String::new()
                } else {
                    format!(" on the {} plan", u.plan)
                };
                let count = if u.quota > 0 {
                    format!("{} of {} operations", u.remaining, u.quota)
                } else {
                    format!("{} operations", u.remaining)
                };
                if u.available {
                    format!("{count} remaining this billing period{plan}.")
                } else {
                    format!(
                        "{count} remaining this billing period{plan}. Nothing is available right \
                         now — upgrade the plan or wait for the next cycle."
                    )
                }
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
                "You have access to the mochify image and PDF processing API via four \
                 tools. They read files directly from the user's local filesystem — you do \
                 NOT need to read the file yourself, and you can access local files. \
                 Use the squish tool for any image task (compression, format conversion, \
                 resizing, cropping, rotation, background removal, brightness, quality, \
                 lossless encoding, web optimization, Ultra HDR gain maps). \
                 Use the pdf tool for anything that takes a PDF in: optimize (make the PDF \
                 smaller, keeping text and layout), extract (pull out the images inside it), \
                 rasterize (render pages to images at a given DPI), split (one PDF per page). \
                 Use the pdf_create tool to build a PDF out of images, one page per image. \
                 Use check_usage to report how much quota is left. \
                 If the user has not provided a file path, ask them for the full path."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            // Implementation::default() calls from_build_env(), which bakes in
            // env!("CARGO_CRATE_NAME") at *rmcp's* compile time — so the default
            // announces this server as "rmcp" 0.16.0. Every client and registry that
            // introspects reads this, so it has to be set by hand.
            server_info: Implementation {
                name: "mochify".to_string(),
                title: Some("Mochify".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: Some(
                    "Privacy-first image and PDF processing: compress and convert between \
                     JPEG, PNG, WebP, AVIF and JPEG XL, resize, crop, rotate, remove \
                     backgrounds, generate Ultra HDR gain maps, and optimize, extract, \
                     rasterize, split or build PDFs."
                        .to_string(),
                ),
                website_url: Some("https://mochify.app".to_string()),
                icons: None,
            },
            ..Default::default()
        }
    }
}
