use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

const BASE_URL: &str = "https://api.mochify.app";
const WORKER_URL: &str = "https://tokens.mochify.app";

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
}

/// Parameters for a `/v1/pdf` request. `op` is "split" (one PDF per page) or
/// "rasterize" (render pages to images); `format`/`dpi`/`quality` apply to rasterize only.
#[derive(Debug, Clone)]
pub struct PdfParams {
    pub op: String,
    pub format: Option<String>,
    pub dpi: Option<u32>,
    pub quality: Option<u32>,
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
        let mut req = self.client.get(format!("{BASE_URL}/v1/checkTokens"));
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }
        let response = req.send().await.context("usage request failed")?;
        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                anyhow::bail!(
                    "Usage tracking requires an API key. \
                     Set MOCHIFY_API_KEY or pass --api-key. \
                     Sign up at https://mochify.app to get one."
                );
            }
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

        let mime = match file_path.extension().and_then(|e| e.to_str()) {
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            Some("avif") => "image/avif",
            Some("jxl") => "image/jxl",
            Some("gif") => "image/gif",
            _ => "application/octet-stream",
        };

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

    /// Resolve a natural-language `prompt` into a `PdfParams` by calling /v1/prompt
    /// with `mode: "pdf"`. The NLP returns a single operation applied to every file
    /// (mirrors the frontend, which sends only filenames for PDFs). Returns the raw
    /// response JSON alongside, for verbose output.
    pub async fn resolve_pdf_prompt(
        &self,
        prompt: &str,
        files: &[&Path],
    ) -> Result<(PdfParams, serde_json::Value)> {
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
            mode: "pdf",
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
            PdfParams {
                op: parsed.pdf.op,
                format: parsed.pdf.format,
                dpi: parsed.pdf.dpi,
                quality: parsed.pdf.quality,
            },
            raw_json,
        ))
    }

    /// POST a PDF to /v1/pdf and save the returned zip (split → page PDFs,
    /// rasterize → page images) to `out_dir`. Returns the written zip path.
    pub async fn pdf(
        &self,
        file_path: &Path,
        params: &PdfParams,
        out_dir: &Path,
    ) -> Result<PathBuf> {
        let bytes = fs::read(file_path)
            .await
            .with_context(|| format!("failed to read {}", file_path.display()))?;

        let query = build_pdf_query(params);

        let mut req = self
            .client
            .post(format!("{BASE_URL}/v1/pdf"))
            .query(&query)
            .header("Content-Type", "application/pdf")
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

        let zip_bytes = response
            .bytes()
            .await
            .context("failed to read response body")?;

        let stem = file_path
            .file_stem()
            .context("invalid file stem")?
            .to_string_lossy();
        let label = if params.op == "split" {
            "pages"
        } else {
            "rasterized"
        };

        // Dedup: if the target already exists, increment until we find a free slot.
        let mut out_path = out_dir.join(format!("{stem}_{label}.zip"));
        if out_path.exists() {
            let mut n = 1u32;
            while out_path.exists() {
                out_path = out_dir.join(format!("{stem}_{label}_{n}.zip"));
                n += 1;
            }
        }

        fs::write(&out_path, &zip_bytes)
            .await
            .with_context(|| format!("failed to write {}", out_path.display()))?;

        Ok(out_path)
    }
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
             Sign up at https://mochify.app to get 25 free requests/month."
        )
    }
}

/// Build the `/v1/pdf` query. `type`/`dpi`/`quality` only apply to rasterize.
fn build_pdf_query(params: &PdfParams) -> Vec<(&'static str, String)> {
    let mut query: Vec<(&'static str, String)> = vec![("op", params.op.clone())];
    if params.op == "rasterize" {
        if let Some(ref t) = params.format {
            query.push(("type", t.clone()));
        }
        if let Some(dpi) = params.dpi {
            query.push(("dpi", dpi.to_string()));
        }
        if let Some(q) = params.quality {
            query.push(("quality", q.to_string()));
        }
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
    query
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
        };
        let q = build_pdf_query(&params);
        let has = |key: &str, val: &str| q.iter().any(|(k, v)| *k == key && v == val);
        assert!(has("op", "rasterize"));
        assert!(has("type", "webp"));
        assert!(has("dpi", "300"));
        assert!(has("quality", "85"));
    }

    #[test]
    fn pdf_prompt_response_deserializes() {
        let json = r#"{"pdf":{"op":"rasterize","type":"png","dpi":150,"quality":92}}"#;
        let parsed: PdfPromptResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.pdf.op, "rasterize");
        assert_eq!(parsed.pdf.format.as_deref(), Some("png"));
        assert_eq!(parsed.pdf.dpi, Some(150));
    }
}
