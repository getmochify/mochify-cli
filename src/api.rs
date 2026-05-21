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
}

#[derive(Deserialize)]
struct SizeEntry {
    width: u32,
    height: u32,
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
            req = req.header("x-api-key", key.as_str());
        }
        let response = req.send().await.context("usage request failed")?;
        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                anyhow::bail!(
                    "Usage tracking requires an API key. \
                     Set MOCHIFY_API_KEY or pass --api-key. \
                     Sign up at https://mochify.app to get one."
                );
            }
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }
        response.json().await.context("failed to parse usage response")
    }

    /// Resolve natural-language `prompt` into per-file `ProcessParams` by calling /v1/prompt.
    /// Returns a map keyed by filename (basename only).
    pub async fn resolve_prompt(
        &self,
        prompt: &str,
        files: &[&Path],
    ) -> Result<HashMap<String, Vec<ProcessParams>>> {
        let mut file_data = Vec::new();
        for &path in files {
            let path_clone = path.to_path_buf();
            let size = tokio::task::spawn_blocking(move || imagesize::size(&path_clone))
                .await?
                .with_context(|| format!("failed to read image dimensions for {}", path.display()))?;
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

        let body = PromptRequest { prompt, file_data };
        let mut req = self
            .client
            .post(format!("{WORKER_URL}/v1/prompt"))
            .json(&body);

        if let Some(ref key) = self.api_key {
            req = req.header("X-Api-Key", key.as_str());
        }

        let response = req.send().await.context("prompt request failed")?;

        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                if self.api_key.is_none() {
                    anyhow::bail!(
                        "Rate limit exceeded. Unauthenticated requests are limited to 3/month per IP. \
                         Sign up at https://mochify.app to get 25 free requests/month."
                    );
                } else {
                    anyhow::bail!(
                        "Rate limit exceeded. You've hit your plan's monthly limit. \
                         Upgrade at https://mochify.app for higher limits (Seller: 300/month, Pro: 1200/month)."
                    );
                }
            }
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let prompt_response: PromptResponse =
            response.json().await.context("failed to parse prompt response")?;

        let mut result: HashMap<String, Vec<ProcessParams>> = HashMap::new();
        for file in prompt_response.files {
            let formats: Vec<String> = match &file.types {
                Some(types) if types.len() > 1 => types.clone(),
                _ => vec![file.format.clone().unwrap_or_else(|| "jpg".to_string())],
            };
            let sizes: Vec<(Option<u32>, Option<u32>)> = match &file.sizes {
                Some(sizes) if sizes.len() > 1 => {
                    sizes.iter().map(|s| (Some(s.width), Some(s.height))).collect()
                }
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
                    let fmt_suffix = if multi_format { format!("_{fmt}") } else { String::new() };
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
                    });
                }
            }
            result.insert(file.filename, variants);
        }
        Ok(result)
    }

    pub async fn squish(
        &self,
        file_path: &Path,
        params: &ProcessParams,
        out_dir: &Path,
    ) -> Result<PathBuf> {
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

        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(ref fmt) = params.format {
            query.push(("type", fmt.clone()));
        }
        if let Some(w) = params.width {
            query.push(("width", w.to_string()));
        }
        if let Some(h) = params.height {
            query.push(("height", h.to_string()));
        }
        if let Some(c) = params.crop {
            query.push(("crop", c.to_string()));
        }
        if let Some(r) = params.rotation {
            query.push(("rotate", r.to_string()));
        }

        let mut req = self
            .client
            .post(format!("{BASE_URL}/v1/squish"))
            .query(&query)
            .header("Content-Type", mime)
            .body(bytes);

        if let Some(ref key) = self.api_key {
            req = req.header("x-api-key", key.as_str());
        }

        let response = req.send().await.context("request failed")?;

        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                if self.api_key.is_none() {
                    anyhow::bail!(
                        "Rate limit exceeded. Unauthenticated requests are limited to 3/month per IP. \
                         Sign up at https://mochify.app to get 25 free requests/month."
                    );
                } else {
                    anyhow::bail!(
                        "Rate limit exceeded. You've hit your plan's monthly limit. \
                         Upgrade at https://mochify.app for higher limits (Seller: 300/month, Pro: 1200/month)."
                    );
                }
            }
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let image_bytes = response.bytes().await.context("failed to read response body")?;

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

        // Multi-variant jobs carry an explicit suffix (e.g. "_500w_webp").
        // Single-variant jobs that would overwrite the input get _mochified instead.
        let candidate = out_dir.join(format!("{stem}.{ext}"));
        let base_stem = if let Some(ref suffix) = params.out_name_suffix {
            format!("{stem}{suffix}")
        } else if candidate == file_path {
            format!("{stem}_mochified")
        } else {
            stem.to_string()
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

        Ok(out_path)
    }
}
