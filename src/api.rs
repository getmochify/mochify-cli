use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

const BASE_URL: &str = "https://api.mochify.app";
const WORKER_URL: &str = "https://id.mochify.app";

#[derive(Debug, Default, Clone)]
pub struct ProcessParams {
    pub format: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub crop: Option<bool>,
    pub rotation: Option<u32>,
    /// Suffix appended to the output filename stem for multi-variant jobs (e.g. "_500w_webp").
    pub out_name_suffix: Option<String>,
    /// Explicit output base name (without extension). Overrides the input filename stem.
    pub output_name: Option<String>,
    pub clarity: Option<bool>,
    pub remove_background: Option<bool>,
    pub background: Option<String>,
    /// Quality override 1-100. Overrides smart compression; `lossless` overrides it.
    pub quality: Option<u32>,
    /// Saliency-guided quality selection: detailed subjects get more, flat areas less.
    pub smart_compress: Option<bool>,
    /// Exposure adjustment, -100 (darkest) to +100 (brightest).
    pub brightness: Option<i32>,
    /// Progressive encoding plus 4:2:0 chroma subsampling, for the smallest web file.
    pub optimize_for_web: Option<bool>,
    /// Pixel-exact output. Only jxl, webp and png can honour it — the API rejects
    /// jpg and avif with a 400 rather than quietly encoding them lossy.
    pub lossless: Option<bool>,
    /// EXIF/metadata handling. The API strips by default; `Some(false)` preserves it.
    pub strip_exif: Option<bool>,
    /// Ultra HDR / ISO 21496-1 gain map handling, already normalised to a wire value:
    /// `"1"` preserves a gain map the source already has, `"generate"` also synthesises
    /// one for a plain SDR source. Only JPEG output can carry a gain map.
    pub hdr: Option<String>,
}

/// Parameters for a `/v1/pdf` request. `op` selects the operation and decides which of
/// the remaining fields are sent on the wire — see `build_pdf_query`.
#[derive(Debug, Clone, Default)]
pub struct PdfParams {
    /// `optimize` | `extract` | `rasterize` | `split` | `create`.
    pub op: String,
    /// Output image format: rasterize (`png|jpg|webp|avif|jxl`), extract (those plus `original`).
    pub format: Option<String>,
    /// Render resolution for rasterize; the DPI a `create` `fit` page is sized at.
    pub dpi: Option<u32>,
    /// Output quality 1-100 (optimize, extract, rasterize, create).
    pub quality: Option<u32>,
    /// optimize: target resolution, measured against how large each image is drawn.
    pub max_dpi: Option<u32>,
    /// optimize: hard cap on the longest side of any image rewritten into the PDF.
    pub max_dimension: Option<u32>,
    /// extract/create: cap each image's width, preserving aspect ratio.
    pub max_width: Option<u32>,
    /// optimize/extract: leave images smaller than this (on either axis) untouched.
    pub min_size: Option<u32>,
    /// create: page size — `fit` | `a4` | `letter`.
    pub page: Option<String>,
    /// create: `Some(false)` returns one single-page PDF per image, as a zip.
    pub combine: Option<bool>,
}

/// The generic inputs a caller collects (CLI flags, MCP tool arguments) before they are
/// mapped onto the query parameters the chosen op actually understands.
#[derive(Debug, Default, Clone)]
pub struct PdfOptions {
    pub format: Option<String>,
    pub dpi: Option<u32>,
    pub quality: Option<u32>,
    pub max_width: Option<u32>,
    pub min_size: Option<u32>,
    pub page: Option<String>,
    pub combine: Option<bool>,
}

impl PdfParams {
    /// Validate `op` and map `opts` onto it. One `dpi` and one `max_width` cover all five
    /// ops, because that is how a person thinks about them — "what resolution" and "how
    /// wide" — while the API spells each differently per op (`dpi` vs `maxDpi`, `maxWidth`
    /// vs `maxDimension`). This is the same mapping the web app makes from its own NLP.
    pub fn for_op(op: &str, opts: PdfOptions) -> Result<Self> {
        let op = op.trim().to_lowercase();
        match op.as_str() {
            // split takes no parameters at all: one single-page PDF per page.
            "split" => Ok(PdfParams {
                op,
                ..Default::default()
            }),
            // Defaults match the web app's, so `--op rasterize` alone does something sensible.
            "rasterize" => Ok(PdfParams {
                format: Some(pdf_format(&op, opts.format)?.unwrap_or_else(|| "png".to_string())),
                dpi: Some(opts.dpi.unwrap_or(150)),
                quality: opts.quality,
                op,
                ..Default::default()
            }),
            "extract" => Ok(PdfParams {
                format: pdf_format(&op, opts.format)?,
                quality: opts.quality,
                max_width: opts.max_width,
                min_size: opts.min_size,
                op,
                ..Default::default()
            }),
            // optimize rewrites images back into the PDF, so it has no output format:
            // the resolution is a target for the images kept inside it, and the width
            // cap is a backstop on their longest side.
            "optimize" => Ok(PdfParams {
                quality: opts.quality,
                max_dpi: opts.dpi,
                max_dimension: opts.max_width,
                min_size: opts.min_size,
                op,
                ..Default::default()
            }),
            "create" => {
                let page = match opts.page {
                    Some(p) => {
                        let p = p.trim().to_lowercase();
                        if !matches!(p.as_str(), "fit" | "a4" | "letter") {
                            anyhow::bail!("Unknown page size '{p}'. Use fit, a4, or letter.");
                        }
                        Some(p)
                    }
                    None => None,
                };
                Ok(PdfParams {
                    quality: opts.quality,
                    dpi: opts.dpi,
                    max_width: opts.max_width,
                    page,
                    combine: opts.combine,
                    op,
                    ..Default::default()
                })
            }
            _ => anyhow::bail!(
                "Unknown PDF operation '{op}'. Use optimize, extract, rasterize or split \
                 for a PDF, or create to build a PDF from images."
            ),
        }
    }
}

/// Normalise and check a PDF image format. `extract` additionally accepts `original`,
/// which copies each embedded image out with no re-encode.
fn pdf_format(op: &str, format: Option<String>) -> Result<Option<String>> {
    let Some(raw) = format else {
        return Ok(None);
    };
    let normalized = match raw.trim().to_lowercase().as_str() {
        "jpeg" => "jpg".to_string(),
        other => other.to_string(),
    };
    let allowed: &[&str] = if op == "extract" {
        &["original", "png", "jpg", "webp", "avif", "jxl"]
    } else {
        &["png", "jpg", "webp", "avif", "jxl"]
    };
    if !allowed.contains(&normalized.as_str()) {
        anyhow::bail!(
            "Unsupported type '{raw}' for op {op}. Use one of: {}.",
            allowed.join(", ")
        );
    }
    Ok(Some(normalized))
}

/// What `/v1/prompt` resolved a PDF instruction to, before CLI flags are merged in.
/// `dpi` and `max_width` are the NLP's generic "resolution" and "size cap": which query
/// parameter each becomes depends on the op, and is decided when the `PdfParams` is built.
#[derive(Debug, Clone)]
pub struct PdfPrompt {
    pub op: String,
    pub format: Option<String>,
    pub dpi: Option<u32>,
    pub quality: Option<u32>,
    pub max_width: Option<u32>,
    pub page: Option<String>,
    pub combine: Option<bool>,
}

#[derive(Deserialize)]
struct SizeEntry {
    width: u32,
    height: u32,
}

#[derive(Serialize)]
struct PdfPromptFileData {
    name: String,
}

#[derive(Serialize)]
struct PdfPromptRequest<'a> {
    prompt: &'a str,
    #[serde(rename = "fileData")]
    file_data: Vec<PdfPromptFileData>,
    mode: &'a str,
}

#[derive(Deserialize)]
struct PdfPromptResult {
    op: String,
    #[serde(rename = "type")]
    format: Option<String>,
    dpi: Option<u32>,
    quality: Option<u32>,
    #[serde(rename = "maxWidth")]
    max_width: Option<u32>,
    /// `imgpdf` mode only (op = create).
    page: Option<String>,
    /// `imgpdf` mode only (op = create).
    combine: Option<bool>,
}

#[derive(Deserialize)]
struct PdfPromptResponse {
    pdf: PdfPromptResult,
}

#[derive(Serialize)]
struct PromptFileData {
    name: String,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
struct PromptRequest<'a> {
    prompt: &'a str,
    #[serde(rename = "fileData")]
    file_data: Vec<PromptFileData>,
}

#[derive(Deserialize)]
pub struct UsageInfo {
    pub remaining: i32,
    #[serde(default)]
    pub quota: i32,
    #[serde(default)]
    pub plan: String,
    pub available: bool,
}

#[derive(Debug, Default)]
pub struct SquishMeta {
    pub latency_ms: Option<String>,
    pub optimized: bool,
    pub reason: Option<String>,
    pub quality: Option<String>,
    pub saliency: Option<String>,
    pub bg_removed: bool,
    /// `X-Mochify-HDR`: `true` (the source's own headroom), `generated` (synthesised),
    /// or `false` (the returned bytes carry none). Only sent when `hdr` was requested.
    pub hdr: Option<String>,
    /// `X-Mochify-Lossless`: `true` (pixel-exact) or `downgraded` (the best lossy encode,
    /// because the source was already lossy). Only sent when `lossless` was requested.
    pub lossless: Option<String>,
}

/// Headers `/v1/pdf` reports back. Which are present depends on the op.
#[derive(Debug, Default)]
pub struct PdfMeta {
    pub latency_ms: Option<String>,
    pub pages: Option<String>,
    pub images: Option<String>,
    pub recompressed: Option<String>,
    pub saved_pct: Option<String>,
}

#[derive(Deserialize)]
struct PromptFileResult {
    filename: String,
    #[serde(rename = "type")]
    format: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    crop: Option<bool>,
    #[serde(default)]
    rotate: u32,
    #[serde(rename = "outputName")]
    output_name: Option<String>,
    clarity: Option<bool>,
    #[serde(rename = "removeBackground")]
    remove_background: Option<bool>,
    /// Composite background colour (e.g. "white", "#ff0000") when the prompt specifies one.
    background: Option<String>,
    /// The NLP returns a plain boolean; true means "give me HDR", which maps to
    /// `hdr=generate` (preserve an existing gain map, synthesise one otherwise).
    hdr: Option<bool>,
    quality: Option<u32>,
    #[serde(rename = "smartCompress")]
    smart_compress: Option<bool>,
    /// The NLP always emits this one, as 0 when no exposure change was asked for.
    brightness: Option<i32>,
    #[serde(rename = "optimizeForWeb")]
    optimize_for_web: Option<bool>,
    lossless: Option<bool>,
    /// Multi-format: set when NLP returns more than one output format.
    types: Option<Vec<String>>,
    /// Multi-size: set when NLP returns more than one output size.
    sizes: Option<Vec<SizeEntry>>,
}

#[derive(Deserialize)]
struct PromptResponse {
    files: Vec<PromptFileResult>,
}

pub struct MochifyClient {
    api_key: Option<String>,
    client: reqwest::Client,
}

impl MochifyClient {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }

    pub async fn get_usage(&self) -> Result<UsageInfo> {
        // `/v1/usage` returns anonymous IP-based quota when unauthenticated, so gate
        // locally: `mochify usage` reports *your account's* usage and needs a key.
        let Some(key) = self.api_key.as_ref() else {
            anyhow::bail!(
                "Usage tracking requires authentication. \
                 Run `mochify auth login` to sign in, \
                 or set MOCHIFY_API_KEY / pass --api-key for automation. \
                 Sign up at https://mochify.app if you don't have an account."
            );
        };
        let response = self
            .client
            .get(format!("{WORKER_URL}/v1/usage"))
            .bearer_auth(key)
            .send()
            .await
            .context("usage request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }
        response
            .json()
            .await
            .context("failed to parse usage response")
    }

    /// Resolve natural-language `prompt` into per-file `ProcessParams` by calling /v1/prompt.
    /// Returns a map keyed by filename (basename only), plus the raw response JSON for verbose output.
    pub async fn resolve_prompt(
        &self,
        prompt: &str,
        files: &[&Path],
    ) -> Result<(HashMap<String, Vec<ProcessParams>>, serde_json::Value)> {
        let mut file_data = Vec::new();
        for &path in files {
            let path_clone = path.to_path_buf();
            let size = tokio::task::spawn_blocking(move || imagesize::size(&path_clone))
                .await?
                .with_context(|| {
                    format!("failed to read image dimensions for {}", path.display())
                })?;
            let name = path
                .file_name()
                .context("invalid filename")?
                .to_string_lossy()
                .into_owned();
            file_data.push(PromptFileData {
                name,
                width: size.width as u32,
                height: size.height as u32,
            });
        }

        let input_names: Vec<String> = file_data.iter().map(|f| f.name.clone()).collect();
        let body = PromptRequest { prompt, file_data };
        let mut req = self
            .client
            .post(format!("{WORKER_URL}/v1/prompt"))
            .json(&body);

        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let response = req.send().await.context("prompt request failed")?;

        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(rate_limit_error(self.api_key.is_some()));
            }
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let body_text = response
            .text()
            .await
            .context("failed to read prompt response")?;
        let raw_json: serde_json::Value =
            serde_json::from_str(&body_text).context("failed to parse prompt response")?;
        let prompt_response: PromptResponse =
            serde_json::from_value(raw_json.clone()).context("failed to parse prompt response")?;

        let mut result: HashMap<String, Vec<ProcessParams>> = HashMap::new();
        for (i, file) in prompt_response.files.into_iter().enumerate() {
            let variants = expand_file_variants(&file);
            // Key by the original input name (not the AI-returned filename) so that
            // files with spaces are always found regardless of how Mistral echoes the name.
            let key = input_names.get(i).cloned().unwrap_or(file.filename);
            result.insert(key, variants);
        }
        Ok((result, raw_json))
    }

    pub async fn squish(
        &self,
        file_path: &Path,
        params: &ProcessParams,
        out_dir: &Path,
    ) -> Result<(PathBuf, SquishMeta)> {
        let bytes = fs::read(file_path)
            .await
            .with_context(|| format!("failed to read {}", file_path.display()))?;

        let mime = image_mime(file_path);

        let query = build_squish_query(params);

        let mut req = self
            .client
            .post(format!("{BASE_URL}/v1/squish"))
            .query(&query)
            .header("Content-Type", mime)
            .body(bytes);

        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let response = req.send().await.context("request failed")?;

        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(rate_limit_error(self.api_key.is_some()));
            }
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let hdr = |name: &str| -> Option<String> {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        };
        let meta = SquishMeta {
            latency_ms: hdr("x-latency-ms"),
            optimized: hdr("x-mochify-optimized")
                .map(|v| v == "true")
                .unwrap_or(false),
            reason: hdr("x-mochify-reason"),
            quality: hdr("x-mochify-quality"),
            saliency: hdr("x-mochify-saliency"),
            bg_removed: hdr("x-mochify-bgremoved")
                .map(|v| v == "true")
                .unwrap_or(false),
            hdr: hdr("x-mochify-hdr"),
            lossless: hdr("x-mochify-lossless"),
        };

        let image_bytes = response
            .bytes()
            .await
            .context("failed to read response body")?;

        let stem = file_path
            .file_stem()
            .context("invalid file stem")?
            .to_string_lossy();

        let ext = params.format.as_deref().unwrap_or(
            file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("jpg"),
        );

        // Resolve the base name: explicit output_name wins over input stem.
        let base_name: String = match &params.output_name {
            Some(name) => sanitize_output_name(name),
            None => stem.to_string(),
        };
        // Multi-variant jobs carry an explicit suffix (e.g. "_500w_webp").
        // Single-variant jobs that would overwrite the input get _mochified instead.
        let candidate = out_dir.join(format!("{stem}.{ext}"));
        let base_stem = if let Some(ref suffix) = params.out_name_suffix {
            format!("{base_name}{suffix}")
        } else if params.output_name.is_none() && candidate == file_path {
            format!("{stem}_mochified")
        } else {
            base_name
        };

        // Dedup: if the target already exists, increment until we find a free slot.
        let mut out_path = out_dir.join(format!("{base_stem}.{ext}"));
        if out_path.exists() {
            let mut n = 1u32;
            while out_path.exists() {
                out_path = out_dir.join(format!("{base_stem}_{n}.{ext}"));
                n += 1;
            }
        }

        fs::write(&out_path, &image_bytes)
            .await
            .with_context(|| format!("failed to write {}", out_path.display()))?;

        Ok((out_path, meta))
    }

    /// Resolve a natural-language `prompt` into a `PdfPrompt` by calling /v1/prompt.
    /// `mode` is "pdf" for PDF inputs (op = optimize/extract/rasterize/split) or
    /// "imgpdf" for images being built into a PDF (op = create). The NLP returns a
    /// single operation applied to every file (mirrors the frontend, which sends only
    /// filenames here). Returns the raw response JSON alongside, for verbose output.
    pub async fn resolve_pdf_prompt(
        &self,
        prompt: &str,
        files: &[&Path],
        mode: &str,
    ) -> Result<(PdfPrompt, serde_json::Value)> {
        let file_data = files
            .iter()
            .map(|p| PdfPromptFileData {
                name: p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            })
            .collect();

        let body = PdfPromptRequest {
            prompt,
            file_data,
            mode,
        };
        let mut req = self
            .client
            .post(format!("{WORKER_URL}/v1/prompt"))
            .json(&body);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let response = req.send().await.context("prompt request failed")?;
        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(rate_limit_error(self.api_key.is_some()));
            }
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let body_text = response
            .text()
            .await
            .context("failed to read prompt response")?;
        let raw_json: serde_json::Value =
            serde_json::from_str(&body_text).context("failed to parse prompt response")?;
        let parsed: PdfPromptResponse = serde_json::from_value(raw_json.clone())
            .context("prompt did not resolve to a PDF operation")?;

        Ok((
            PdfPrompt {
                op: parsed.pdf.op,
                format: parsed.pdf.format,
                dpi: parsed.pdf.dpi,
                quality: parsed.pdf.quality,
                max_width: parsed.pdf.max_width,
                page: parsed.pdf.page,
                combine: parsed.pdf.combine,
            },
            raw_json,
        ))
    }

    /// POST a PDF to /v1/pdf and save what comes back to `out_dir`: `optimize` returns a
    /// smaller PDF, every other PDF-in op (`extract`, `rasterize`, `split`) returns a zip.
    /// Returns the written path and the headers the op reported.
    pub async fn pdf(
        &self,
        file_path: &Path,
        params: &PdfParams,
        out_dir: &Path,
    ) -> Result<(PathBuf, PdfMeta)> {
        let bytes = fs::read(file_path)
            .await
            .with_context(|| format!("failed to read {}", file_path.display()))?;

        let req = self
            .client
            .post(format!("{BASE_URL}/v1/pdf"))
            .query(&build_pdf_query(params))
            .header("Content-Type", "application/pdf")
            .body(bytes);

        let (result_bytes, meta) = self.send_pdf(req).await?;

        let stem = file_path
            .file_stem()
            .context("invalid file stem")?
            .to_string_lossy()
            .into_owned();
        let (base, ext) = pdf_output_name(&params.op, &stem, params.combine);
        let out_path = unique_path(out_dir, &base, ext);

        fs::write(&out_path, &result_bytes)
            .await
            .with_context(|| format!("failed to write {}", out_path.display()))?;

        Ok((out_path, meta))
    }

    /// POST images to /v1/pdf?op=create as `multipart/form-data` (one `images` part per
    /// file, in order) and save the result: one combined PDF, or a zip of single-page
    /// PDFs when `params.combine` is `Some(false)`. `output_name` overrides the base
    /// name, which otherwise comes from the first image.
    pub async fn pdf_create(
        &self,
        files: &[PathBuf],
        params: &PdfParams,
        out_dir: &Path,
        output_name: Option<&str>,
    ) -> Result<(PathBuf, PdfMeta)> {
        let mut form = reqwest::multipart::Form::new();
        for path in files {
            let bytes = fs::read(path)
                .await
                .with_context(|| format!("failed to read {}", path.display()))?;
            let name = path
                .file_name()
                .context("invalid filename")?
                .to_string_lossy()
                .into_owned();
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(name)
                .mime_str(image_mime(path))
                .context("invalid image content type")?;
            form = form.part("images", part);
        }

        // No explicit Content-Type: reqwest sets it with the multipart boundary.
        let req = self
            .client
            .post(format!("{BASE_URL}/v1/pdf"))
            .query(&build_pdf_query(params))
            .multipart(form);

        let (result_bytes, meta) = self.send_pdf(req).await?;

        let first = files.first().context("no input images")?;
        let stem = match output_name {
            Some(name) => sanitize_output_name(name),
            None => first
                .file_stem()
                .context("invalid file stem")?
                .to_string_lossy()
                .into_owned(),
        };
        let (base, ext) = pdf_output_name(&params.op, &stem, params.combine);
        let out_path = unique_path(out_dir, &base, ext);

        fs::write(&out_path, &result_bytes)
            .await
            .with_context(|| format!("failed to write {}", out_path.display()))?;

        Ok((out_path, meta))
    }

    /// Authenticate, send, and turn a /v1/pdf response into bytes + headers.
    async fn send_pdf(&self, req: reqwest::RequestBuilder) -> Result<(Vec<u8>, PdfMeta)> {
        let req = match self.api_key {
            Some(ref key) => req.bearer_auth(key),
            None => req,
        };

        let response = req.send().await.context("request failed")?;
        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(rate_limit_error(self.api_key.is_some()));
            }
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let header = |name: &str| -> Option<String> {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        };
        let meta = PdfMeta {
            latency_ms: header("x-latency-ms"),
            pages: header("x-mochify-pages"),
            images: header("x-mochify-images"),
            recompressed: header("x-mochify-images-recompressed"),
            saved_pct: header("x-mochify-saved-pct"),
        };

        let body = response
            .bytes()
            .await
            .context("failed to read response body")?;
        Ok((body.to_vec(), meta))
    }
}

/// Content type for an image upload, by extension.
fn image_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("jxl") => "image/jxl",
        Some("gif") => "image/gif",
        Some("heic") | Some("heif") => "image/heic",
        _ => "application/octet-stream",
    }
}

/// Output base name and extension for a PDF op. `optimize` and a combined `create`
/// hand back a single PDF; every other op hands back an archive of many files.
fn pdf_output_name(op: &str, stem: &str, combine: Option<bool>) -> (String, &'static str) {
    match op {
        "optimize" => (format!("{stem}_compressed"), "pdf"),
        "extract" => (format!("{stem}_images"), "zip"),
        "split" => (format!("{stem}_pages"), "zip"),
        "create" if combine == Some(false) => (format!("{stem}_pdfs"), "zip"),
        "create" => (stem.to_string(), "pdf"),
        _ => (format!("{stem}_rasterized"), "zip"),
    }
}

/// `out_dir/base.ext`, with a numeric suffix if that name is already taken.
fn unique_path(out_dir: &Path, base: &str, ext: &str) -> PathBuf {
    let mut out_path = out_dir.join(format!("{base}.{ext}"));
    let mut n = 1u32;
    while out_path.exists() {
        out_path = out_dir.join(format!("{base}_{n}.{ext}"));
        n += 1;
    }
    out_path
}

/// Map a non-success rate-limit status to a helpful, plan-aware error.
fn rate_limit_error(has_key: bool) -> anyhow::Error {
    if has_key {
        anyhow::anyhow!(
            "Rate limit exceeded. You've hit your plan's monthly limit. \
             Upgrade at https://mochify.app for higher limits (Seller: 300/month, Pro: 1200/month)."
        )
    } else {
        anyhow::anyhow!(
            "Rate limit exceeded. Unauthenticated requests are limited to 3/month per IP. \
             Sign up at https://mochify.app, then run `mochify auth login` for 25 free requests/month."
        )
    }
}

/// Build the `/v1/pdf` query. Each op takes its own set of parameters, and the API
/// rejects or ignores the others, so only the ones that op understands are sent.
/// `split` takes none at all.
fn build_pdf_query(params: &PdfParams) -> Vec<(&'static str, String)> {
    let mut query: Vec<(&'static str, String)> = vec![("op", params.op.clone())];
    let mut push = |key: &'static str, value: Option<u32>| {
        // 0 is meaningful here (it disables a cap), so Some(0) is still sent.
        if let Some(v) = value {
            query.push((key, v.to_string()));
        }
    };
    match params.op.as_str() {
        "rasterize" => {
            push("dpi", params.dpi);
            push("quality", params.quality);
            if let Some(ref t) = params.format {
                query.push(("type", t.clone()));
            }
        }
        "extract" => {
            push("quality", params.quality);
            push("maxWidth", params.max_width);
            push("minSize", params.min_size);
            if let Some(ref t) = params.format {
                query.push(("type", t.clone()));
            }
        }
        "optimize" => {
            push("quality", params.quality);
            push("maxDpi", params.max_dpi);
            push("maxDimension", params.max_dimension);
            push("minSize", params.min_size);
        }
        "create" => {
            push("quality", params.quality);
            push("dpi", params.dpi);
            push("maxWidth", params.max_width);
            if let Some(ref page) = params.page {
                query.push(("page", page.clone()));
            }
            if let Some(combine) = params.combine {
                query.push(("combine", if combine { "1" } else { "0" }.to_string()));
            }
        }
        // split: the whole document, one PDF per page, nothing to configure.
        _ => {}
    }
    query
}

/// Expand a single NLP file result into one `ProcessParams` per (size × format) variant.
/// Jobs with more than one format or size get an output-name suffix (`_500w`, `_1000x1000`,
/// `_webp`, or a combination); single-variant jobs carry no suffix.
fn expand_file_variants(file: &PromptFileResult) -> Vec<ProcessParams> {
    let formats: Vec<String> = match &file.types {
        Some(types) if types.len() > 1 => types.clone(),
        _ => vec![file.format.clone().unwrap_or_else(|| "jpg".to_string())],
    };
    let sizes: Vec<(Option<u32>, Option<u32>)> = match &file.sizes {
        Some(sizes) if sizes.len() > 1 => sizes
            .iter()
            .map(|s| (Some(s.width), Some(s.height)))
            .collect(),
        _ => vec![(file.width, file.height)],
    };

    let multi_format = formats.len() > 1;
    let multi_size = sizes.len() > 1;

    let mut variants = Vec::new();
    for (w, h) in &sizes {
        for fmt in &formats {
            let size_suffix = if multi_size {
                match (w.filter(|&v| v > 0), h.filter(|&v| v > 0)) {
                    (Some(w), Some(h)) => format!("_{w}x{h}"),
                    (Some(w), _) => format!("_{w}w"),
                    (_, Some(h)) => format!("_{h}h"),
                    _ => String::new(),
                }
            } else {
                String::new()
            };
            let fmt_suffix = if multi_format {
                format!("_{fmt}")
            } else {
                String::new()
            };
            let out_name_suffix = if multi_format || multi_size {
                Some(format!("{size_suffix}{fmt_suffix}"))
            } else {
                None
            };
            variants.push(ProcessParams {
                format: Some(fmt.clone()),
                width: *w,
                height: *h,
                crop: file.crop,
                rotation: (file.rotate != 0).then_some(file.rotate),
                out_name_suffix,
                output_name: file.output_name.clone(),
                clarity: file.clarity,
                remove_background: file.remove_background,
                background: file.background.clone(),
                strip_exif: None,
                // The NLP answers with a boolean; "generate" is the mode that also
                // synthesises a gain map for an SDR source, which is what someone
                // asking for HDR in words means (and matches the web app).
                hdr: file.hdr.and_then(|on| on.then(|| "generate".to_string())),
                quality: file.quality,
                // The NLP sends these as required booleans, false far more often than
                // true, so only a true is worth putting on the wire.
                smart_compress: file.smart_compress.filter(|&on| on),
                brightness: file.brightness.filter(|&b| b != 0),
                optimize_for_web: file.optimize_for_web.filter(|&on| on),
                lossless: file.lossless.filter(|&on| on),
            });
        }
    }
    variants
}

/// Build the `/v1/squish` query from resolved params. Zero-valued width/height are dropped —
/// the NLP can echo 0 to mean "unspecified", and forwarding it would resize to nothing.
fn build_squish_query(params: &ProcessParams) -> Vec<(&'static str, String)> {
    let mut query: Vec<(&'static str, String)> = Vec::new();
    if let Some(ref fmt) = params.format {
        query.push(("type", fmt.clone()));
    }
    if let Some(w) = params.width.filter(|&w| w > 0) {
        query.push(("width", w.to_string()));
    }
    if let Some(h) = params.height.filter(|&h| h > 0) {
        query.push(("height", h.to_string()));
    }
    if let Some(c) = params.crop {
        query.push(("crop", c.to_string()));
    }
    if let Some(r) = params.rotation {
        query.push(("rotate", r.to_string()));
    }
    if params.clarity == Some(true) {
        query.push(("clarity", "1".to_string()));
    }
    if params.remove_background == Some(true) {
        query.push(("removeBackground", "1".to_string()));
    }
    if let Some(ref bg) = params.background {
        query.push(("background", bg.clone()));
    }
    // The API strips EXIF by default; only send the param when explicitly set.
    if let Some(strip) = params.strip_exif {
        query.push(("stripExif", strip.to_string()));
    }
    if let Some(ref hdr) = params.hdr {
        query.push(("hdr", hdr.clone()));
    }
    if let Some(q) = params.quality {
        query.push(("quality", q.to_string()));
    }
    if params.smart_compress == Some(true) {
        query.push(("smartCompress", "1".to_string()));
    }
    // 0 is "no change", which is what omitting it already means.
    if let Some(b) = params.brightness.filter(|&b| b != 0) {
        query.push(("brightness", b.to_string()));
    }
    if params.optimize_for_web == Some(true) {
        query.push(("optimizeForWeb", "1".to_string()));
    }
    if params.lossless == Some(true) {
        query.push(("lossless", "1".to_string()));
    }
    query
}

/// Formats that can hold pixel-exact output. The API answers `lossless` with jpg or
/// avif with a 400, so the CLI and the MCP tools check it before spending a request.
pub const LOSSLESS_FORMATS: [&str; 3] = ["jxl", "webp", "png"];

/// Normalise a user-supplied HDR mode to the wire value. `preserve` keeps a gain map
/// the source already has (and does nothing to an SDR source); `generate` also
/// synthesises one. Anything else is a typo worth failing on rather than guessing.
pub fn normalize_hdr_mode(mode: &str) -> Result<String> {
    match mode.trim().to_lowercase().as_str() {
        "preserve" | "keep" | "1" | "true" | "on" | "yes" => Ok("1".to_string()),
        "generate" | "gen" | "make" | "synthesize" | "synthesise" => Ok("generate".to_string()),
        other => anyhow::bail!(
            "Unknown --hdr mode '{other}'. Use 'preserve' (keep a gain map the source has) \
             or 'generate' (also create one for an SDR source)."
        ),
    }
}

/// Strip characters invalid in filenames and trim surrounding whitespace.
fn sanitize_output_name(name: &str) -> String {
    name.chars()
        .filter(|c| {
            !matches!(
                c,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t'
            )
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_result() -> PromptFileResult {
        PromptFileResult {
            filename: "photo.jpg".into(),
            format: Some("webp".into()),
            width: Some(800),
            height: None,
            crop: None,
            rotate: 0,
            output_name: None,
            clarity: None,
            remove_background: None,
            background: None,
            hdr: None,
            quality: None,
            smart_compress: None,
            brightness: None,
            optimize_for_web: None,
            lossless: None,
            types: None,
            sizes: None,
        }
    }

    #[test]
    fn single_variant_has_no_suffix() {
        let v = expand_file_variants(&sample_result());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].format.as_deref(), Some("webp"));
        assert_eq!(v[0].width, Some(800));
        assert_eq!(v[0].out_name_suffix, None);
    }

    #[test]
    fn multi_format_suffixes_each_variant() {
        let mut f = sample_result();
        f.types = Some(vec!["webp".into(), "avif".into()]);
        let v = expand_file_variants(&f);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].out_name_suffix.as_deref(), Some("_webp"));
        assert_eq!(v[1].out_name_suffix.as_deref(), Some("_avif"));
    }

    #[test]
    fn multi_size_uses_dimension_suffix() {
        let mut f = sample_result();
        f.sizes = Some(vec![
            SizeEntry {
                width: 500,
                height: 0,
            },
            SizeEntry {
                width: 1000,
                height: 1000,
            },
        ]);
        let v = expand_file_variants(&f);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].out_name_suffix.as_deref(), Some("_500w"));
        assert_eq!(v[1].out_name_suffix.as_deref(), Some("_1000x1000"));
    }

    #[test]
    fn multi_size_and_format_combine_suffixes() {
        let mut f = sample_result();
        f.types = Some(vec!["webp".into(), "avif".into()]);
        f.sizes = Some(vec![
            SizeEntry {
                width: 500,
                height: 0,
            },
            SizeEntry {
                width: 1000,
                height: 0,
            },
        ]);
        let v = expand_file_variants(&f);
        assert_eq!(v.len(), 4); // 2 sizes × 2 formats
        assert_eq!(v[0].out_name_suffix.as_deref(), Some("_500w_webp"));
        assert_eq!(v[1].out_name_suffix.as_deref(), Some("_500w_avif"));
        assert_eq!(v[2].out_name_suffix.as_deref(), Some("_1000w_webp"));
        assert_eq!(v[3].out_name_suffix.as_deref(), Some("_1000w_avif"));
    }

    #[test]
    fn rotate_zero_is_omitted_nonzero_kept() {
        assert_eq!(expand_file_variants(&sample_result())[0].rotation, None);
        let mut f = sample_result();
        f.rotate = 90;
        assert_eq!(expand_file_variants(&f)[0].rotation, Some(90));
    }

    #[test]
    fn propagates_remove_background_and_background() {
        let mut f = sample_result();
        f.remove_background = Some(true);
        f.background = Some("white".into());
        let v = expand_file_variants(&f);
        assert_eq!(v[0].remove_background, Some(true));
        assert_eq!(v[0].background.as_deref(), Some("white"));
    }

    #[test]
    fn default_format_is_jpg_when_missing() {
        let mut f = sample_result();
        f.format = None;
        assert_eq!(expand_file_variants(&f)[0].format.as_deref(), Some("jpg"));
    }

    #[test]
    fn deserializes_remove_background_camelcase() {
        let json = r##"{"files":[{"filename":"a.jpg","type":"webp","width":800,"height":600,"removeBackground":true,"background":"#ffffff"}]}"##;
        let parsed: PromptResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.files[0].remove_background, Some(true));
        assert_eq!(parsed.files[0].background.as_deref(), Some("#ffffff"));
    }

    #[test]
    fn build_query_drops_zero_dimensions() {
        let params = ProcessParams {
            format: Some("webp".into()),
            width: Some(0),
            height: Some(0),
            ..Default::default()
        };
        let q = build_squish_query(&params);
        assert!(q.iter().any(|(k, v)| *k == "type" && v == "webp"));
        assert!(!q.iter().any(|(k, _)| *k == "width"));
        assert!(!q.iter().any(|(k, _)| *k == "height"));
    }

    #[test]
    fn strip_exif_only_sent_when_set() {
        // Default (None) — metadata handling left to the API default.
        let none = build_squish_query(&ProcessParams::default());
        assert!(!none.iter().any(|(k, _)| *k == "stripExif"));
        // Explicit preserve.
        let keep = build_squish_query(&ProcessParams {
            strip_exif: Some(false),
            ..Default::default()
        });
        assert!(keep.iter().any(|(k, v)| *k == "stripExif" && v == "false"));
    }

    #[test]
    fn build_query_includes_all_flags() {
        let params = ProcessParams {
            format: Some("png".into()),
            width: Some(1200),
            crop: Some(true),
            rotation: Some(90),
            clarity: Some(true),
            remove_background: Some(true),
            background: Some("white".into()),
            ..Default::default()
        };
        let q = build_squish_query(&params);
        let has = |key: &str, val: &str| q.iter().any(|(k, v)| *k == key && v == val);
        assert!(has("width", "1200"));
        assert!(has("crop", "true"));
        assert!(has("rotate", "90"));
        assert!(has("clarity", "1"));
        assert!(has("removeBackground", "1"));
        assert!(has("background", "white"));
    }

    #[test]
    fn sanitize_strips_invalid_chars_and_trims() {
        assert_eq!(
            sanitize_output_name("  hero/image:final  "),
            "heroimagefinal"
        );
        assert_eq!(sanitize_output_name("logo*?\"<>|"), "logo");
        assert_eq!(sanitize_output_name("clean-name_1"), "clean-name_1");
    }

    #[test]
    fn pdf_split_query_is_op_only() {
        let params = PdfParams {
            op: "split".into(),
            format: Some("png".into()),
            dpi: Some(150),
            quality: Some(90),
            ..Default::default()
        };
        let q = build_pdf_query(&params);
        assert_eq!(q, vec![("op", "split".to_string())]);
    }

    #[test]
    fn pdf_rasterize_query_includes_render_params() {
        let params = PdfParams {
            op: "rasterize".into(),
            format: Some("webp".into()),
            dpi: Some(300),
            quality: Some(85),
            ..Default::default()
        };
        let q = build_pdf_query(&params);
        let has = |key: &str, val: &str| q.iter().any(|(k, v)| *k == key && v == val);
        assert!(has("op", "rasterize"));
        assert!(has("type", "webp"));
        assert!(has("dpi", "300"));
        assert!(has("quality", "85"));
    }

    #[test]
    fn pdf_optimize_query_sends_image_caps_not_type() {
        let params = PdfParams {
            op: "optimize".into(),
            format: Some("webp".into()), // optimize always writes JPEG; type is not a param
            quality: Some(75),
            max_dpi: Some(150),
            max_dimension: Some(2000),
            min_size: Some(0),
            ..Default::default()
        };
        let q = build_pdf_query(&params);
        let has = |key: &str, val: &str| q.iter().any(|(k, v)| *k == key && v == val);
        assert!(has("quality", "75"));
        assert!(has("maxDpi", "150"));
        assert!(has("maxDimension", "2000"));
        assert!(has("minSize", "0")); // 0 disables the floor, so it must still be sent
        assert!(!q.iter().any(|(k, _)| *k == "type"));
    }

    #[test]
    fn pdf_extract_query_sends_type_and_width_cap() {
        let params = PdfParams {
            op: "extract".into(),
            format: Some("original".into()),
            dpi: Some(300), // ignored by extract
            quality: Some(82),
            max_width: Some(1600),
            ..Default::default()
        };
        let q = build_pdf_query(&params);
        let has = |key: &str, val: &str| q.iter().any(|(k, v)| *k == key && v == val);
        assert!(has("type", "original"));
        assert!(has("quality", "82"));
        assert!(has("maxWidth", "1600"));
        assert!(!q.iter().any(|(k, _)| *k == "dpi"));
    }

    #[test]
    fn pdf_create_query_sends_page_and_combine() {
        let params = PdfParams {
            op: "create".into(),
            quality: Some(85),
            dpi: Some(96),
            page: Some("a4".into()),
            combine: Some(false),
            ..Default::default()
        };
        let q = build_pdf_query(&params);
        let has = |key: &str, val: &str| q.iter().any(|(k, v)| *k == key && v == val);
        assert!(has("page", "a4"));
        assert!(has("combine", "0"));
        assert!(has("quality", "85"));
        assert!(has("dpi", "96"));
    }

    #[test]
    fn pdf_output_names_match_what_each_op_returns() {
        assert_eq!(
            pdf_output_name("optimize", "report", None),
            ("report_compressed".to_string(), "pdf")
        );
        assert_eq!(
            pdf_output_name("extract", "report", None),
            ("report_images".to_string(), "zip")
        );
        assert_eq!(
            pdf_output_name("split", "report", None),
            ("report_pages".to_string(), "zip")
        );
        assert_eq!(
            pdf_output_name("rasterize", "report", None),
            ("report_rasterized".to_string(), "zip")
        );
        assert_eq!(
            pdf_output_name("create", "scan", Some(true)),
            ("scan".to_string(), "pdf")
        );
        assert_eq!(
            pdf_output_name("create", "scan", Some(false)),
            ("scan_pdfs".to_string(), "zip")
        );
    }

    #[test]
    fn hdr_modes_normalize_to_wire_values() {
        assert_eq!(normalize_hdr_mode("preserve").unwrap(), "1");
        assert_eq!(normalize_hdr_mode("KEEP").unwrap(), "1");
        assert_eq!(normalize_hdr_mode("true").unwrap(), "1");
        assert_eq!(normalize_hdr_mode("generate").unwrap(), "generate");
        assert!(normalize_hdr_mode("maybe").is_err());
    }

    #[test]
    fn hdr_is_sent_verbatim_and_omitted_when_unset() {
        let q = build_squish_query(&ProcessParams {
            hdr: Some("generate".into()),
            ..Default::default()
        });
        assert!(q.iter().any(|(k, v)| *k == "hdr" && v == "generate"));
        assert!(
            !build_squish_query(&ProcessParams::default())
                .iter()
                .any(|(k, _)| *k == "hdr")
        );
    }

    #[test]
    fn quality_flags_reach_the_query() {
        let q = build_squish_query(&ProcessParams {
            quality: Some(70),
            smart_compress: Some(true),
            brightness: Some(-25),
            optimize_for_web: Some(true),
            lossless: Some(true),
            ..Default::default()
        });
        let has = |key: &str, val: &str| q.iter().any(|(k, v)| *k == key && v == val);
        assert!(has("quality", "70"));
        assert!(has("smartCompress", "1"));
        assert!(has("brightness", "-25"));
        assert!(has("optimizeForWeb", "1"));
        assert!(has("lossless", "1"));
    }

    #[test]
    fn zero_brightness_and_false_flags_are_omitted() {
        let q = build_squish_query(&ProcessParams {
            brightness: Some(0),
            smart_compress: Some(false),
            optimize_for_web: Some(false),
            lossless: Some(false),
            ..Default::default()
        });
        assert!(q.is_empty());
    }

    #[test]
    fn prompt_carries_quality_brightness_and_lossless() {
        // The NLP emits smartCompress/optimizeForWeb on every file, usually false.
        let json = r#"{"files":[{"filename":"a.jpg","type":"webp","width":0,"height":0,
            "quality":72,"brightness":25,"smartCompress":false,"optimizeForWeb":true,
            "lossless":true}]}"#;
        let parsed: PromptResponse = serde_json::from_str(json).unwrap();
        let v = &expand_file_variants(&parsed.files[0])[0];
        assert_eq!(v.quality, Some(72));
        assert_eq!(v.brightness, Some(25));
        assert_eq!(v.smart_compress, None); // false → not sent
        assert_eq!(v.optimize_for_web, Some(true));
        assert_eq!(v.lossless, Some(true));
    }

    #[test]
    fn prompt_hdr_boolean_becomes_generate() {
        let json =
            r#"{"files":[{"filename":"a.jpg","type":"jpg","width":0,"height":0,"hdr":true}]}"#;
        let parsed: PromptResponse = serde_json::from_str(json).unwrap();
        let v = expand_file_variants(&parsed.files[0]);
        assert_eq!(v[0].hdr.as_deref(), Some("generate"));
        // hdr:false must not send the param at all.
        let json =
            r#"{"files":[{"filename":"a.jpg","type":"jpg","width":0,"height":0,"hdr":false}]}"#;
        let parsed: PromptResponse = serde_json::from_str(json).unwrap();
        assert_eq!(expand_file_variants(&parsed.files[0])[0].hdr, None);
    }

    #[test]
    fn pdf_prompt_response_deserializes() {
        let json = r#"{"pdf":{"op":"rasterize","type":"png","dpi":150,"quality":92,"maxWidth":0}}"#;
        let parsed: PdfPromptResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.pdf.op, "rasterize");
        assert_eq!(parsed.pdf.format.as_deref(), Some("png"));
        assert_eq!(parsed.pdf.dpi, Some(150));
        assert_eq!(parsed.pdf.max_width, Some(0));
    }

    #[test]
    fn imgpdf_prompt_response_deserializes() {
        let json = r#"{"pdf":{"op":"create","combine":false,"page":"a4","quality":85}}"#;
        let parsed: PdfPromptResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.pdf.op, "create");
        assert_eq!(parsed.pdf.page.as_deref(), Some("a4"));
        assert_eq!(parsed.pdf.combine, Some(false));
    }
}
