mod api;
mod cli;
mod credentials;
mod mcp;

use anyhow::{Context, Result};
use api::{MochifyClient, PdfParams, ProcessParams, SquishMeta};
use clap::Parser;
use cli::{Args, AuthAction, Commands};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::time::Duration;

const WORKER_URL: &str = "https://tokens.mochify.app";
const AUTH_URL: &str = "https://mochify.app/auth/cli";

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = Args::parse();

    // Fall back to saved credentials if no key was supplied via flag or env.
    if args.api_key.is_none() {
        args.api_key = credentials::load();
    }

    match args.command {
        Some(Commands::Serve) => run_mcp_server(args.api_key).await,
        Some(Commands::Usage) => {
            let client = MochifyClient::new(args.api_key);
            let usage = client.get_usage().await?;
            println!("Remaining: {}", usage.remaining);
            println!("Available: {}", usage.available);
            Ok(())
        }
        Some(Commands::Auth { action }) => match action {
            AuthAction::Login => auth_login().await,
            AuthAction::Logout => auth_logout(),
            AuthAction::Status => auth_status(),
        },
        None => {
            // If no files were given as arguments, try reading paths from stdin
            // (e.g. `find . -name "*.jpg" | mochify -t webp`).
            if args.files.is_empty() {
                use std::io::{self, BufRead};
                if !atty::is(atty::Stream::Stdin) {
                    let stdin = io::stdin();
                    for line in stdin.lock().lines() {
                        let line = line?;
                        let trimmed = line.trim().to_string();
                        if !trimmed.is_empty() {
                            args.files.push(PathBuf::from(trimmed));
                        }
                    }
                }
            }
            if args.files.is_empty() {
                eprintln!("No input files specified. Run with --help for usage.");
                std::process::exit(1);
            }
            process_files(args).await
        }
    }
}

async fn auth_login() -> Result<()> {
    use rand::RngCore;

    let mut state_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut state_bytes);
    let state: String = state_bytes.iter().map(|b| format!("{b:02x}")).collect();

    let url = format!("{AUTH_URL}?state={state}");

    if open::that(&url).is_err() {
        println!("Open this URL in your browser to sign in:");
        println!("  {url}");
    } else {
        println!("Browser opened. Sign in and authorize the CLI...");
    }

    let sp = spinner("Waiting for authorization (times out in 5 minutes)...");
    let client = reqwest::Client::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(300);

    loop {
        if std::time::Instant::now() >= deadline {
            sp.finish_and_clear();
            anyhow::bail!("Authorization timed out. Run `mochify auth login` to try again.");
        }

        tokio::time::sleep(Duration::from_secs(2)).await;

        let res = client
            .get(format!("{WORKER_URL}/v1/cli/poll/{state}"))
            .send()
            .await;

        let Ok(response) = res else { continue };

        match response.status().as_u16() {
            404 => continue,
            200 => {
                #[derive(serde::Deserialize)]
                struct PollResponse {
                    #[serde(rename = "apiKey")]
                    api_key: String,
                }
                sp.finish_and_clear();
                let body = response
                    .json::<PollResponse>()
                    .await
                    .context("authorization succeeded but the response could not be parsed")?;
                credentials::save(&body.api_key)?;
                println!("Authenticated! Credentials saved to ~/.config/mochify/credentials.toml");
                return Ok(());
            }
            _ => continue,
        }
    }
}

fn auth_logout() -> Result<()> {
    credentials::clear()?;
    println!("Credentials removed.");
    Ok(())
}

fn auth_status() -> Result<()> {
    match credentials::load() {
        Some(key) => {
            let preview = &key[..key.len().min(8)];
            println!("Authenticated (key: {preview}…)");
        }
        None => println!("Not authenticated. Run `mochify auth login` to sign in."),
    }
    Ok(())
}

fn is_pdf(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

async fn process_files(args: Args) -> Result<()> {
    let client = MochifyClient::new(args.api_key.clone());

    // PDFs go through /v1/pdf, not /v1/squish. Detect them by extension and route
    // before touching image-only params. A single command can't mix the two modes
    // (the NLP prompt resolves to one mode), mirroring the frontend.
    let has_pdf = args.files.iter().any(|p| is_pdf(p));
    if has_pdf {
        if args.files.iter().any(|p| !is_pdf(p)) {
            anyhow::bail!("Can't mix PDFs and images in one command — run them separately.");
        }
        return process_pdfs(&args, &client).await;
    }

    // Explicit CLI flags — these always win over prompt-derived params.
    let explicit = ProcessParams {
        format: args.format,
        width: args.width,
        height: args.height,
        crop: if args.crop { Some(true) } else { None },
        rotation: args.rotation,
        out_name_suffix: None,
        output_name: args.name,
        clarity: if args.clarity { Some(true) } else { None },
        remove_background: if args.remove_bg { Some(true) } else { None },
        background: None,
    };

    // If a prompt was supplied, resolve params for all files in one request.
    let prompt_map = if let Some(ref prompt) = args.prompt {
        let sp = spinner("Parsing prompt...");
        let paths: Vec<&std::path::Path> = args.files.iter().map(|p| p.as_path()).collect();
        let (map, raw_json) = client.resolve_prompt(prompt, &paths).await?;
        sp.finish_and_clear();
        print_prompt_summary(&args.files, &map);
        if args.verbose {
            eprintln!("Prompt response JSON:");
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&raw_json).unwrap_or_default()
            );
        }
        Some(map)
    } else {
        None
    };

    for file_path in &args.files {
        let out_dir = match &args.output {
            Some(d) => d.clone(),
            None => file_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
        };

        let variants: Vec<ProcessParams> = match &prompt_map {
            Some(map) => {
                let filename = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let base_variants = map
                    .get(filename)
                    .cloned()
                    .unwrap_or_else(|| vec![ProcessParams::default()]);
                base_variants
                    .into_iter()
                    .map(|base| merge_params(base, explicit.clone()))
                    .collect()
            }
            None => vec![explicit.clone()],
        };

        let name = file_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for params in &variants {
            let label = match &params.out_name_suffix {
                Some(s) => format!("{name}{s}"),
                None => name.clone(),
            };
            let sp = spinner(format!("Processing {label}..."));
            match client.squish(file_path, params, &out_dir).await {
                Ok((out, meta)) => {
                    sp.finish_and_clear();
                    println!("{}", out.display());
                    if args.verbose {
                        print_squish_meta(&meta);
                    }
                }
                Err(e) => {
                    sp.finish_and_clear();
                    eprintln!("Error processing {label}: {e:#}");
                }
            }
        }
    }

    Ok(())
}

async fn process_pdfs(args: &Args, client: &MochifyClient) -> Result<()> {
    // Resolve the operation: a prompt (if given) seeds it via NLP, then explicit
    // flags override. Mirrors the image flow's prompt-then-flags precedence.
    let prompt_params = if let Some(ref prompt) = args.prompt {
        let sp = spinner("Parsing prompt...");
        let paths: Vec<&std::path::Path> = args.files.iter().map(|p| p.as_path()).collect();
        let (params, raw_json) = client.resolve_pdf_prompt(prompt, &paths).await?;
        sp.finish_and_clear();
        if args.verbose {
            eprintln!("Prompt response JSON:");
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&raw_json).unwrap_or_default()
            );
        }
        Some(params)
    } else {
        None
    };

    let params = resolve_pdf_params(
        prompt_params,
        args.op.clone(),
        args.format.clone(),
        args.dpi,
        args.quality,
    )?;
    print_pdf_summary(&params);

    for file_path in &args.files {
        let out_dir = match &args.output {
            Some(d) => d.clone(),
            None => file_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
        };

        let name = file_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let sp = spinner(format!("Processing {name}..."));
        match client.pdf(file_path, &params, &out_dir).await {
            Ok(out) => {
                sp.finish_and_clear();
                println!("{}", out.display());
            }
            Err(e) => {
                sp.finish_and_clear();
                eprintln!("Error processing {name}: {e:#}");
            }
        }
    }

    Ok(())
}

/// Combine prompt-derived and explicit PDF params (explicit wins), validate the
/// operation, and apply rasterize defaults (PNG @ 150 DPI, matching the frontend).
fn resolve_pdf_params(
    prompt: Option<PdfParams>,
    op: Option<String>,
    format: Option<String>,
    dpi: Option<u32>,
    quality: Option<u32>,
) -> Result<PdfParams> {
    let op = op
        .or_else(|| prompt.as_ref().map(|p| p.op.clone()))
        .map(|o| o.to_lowercase());
    let op = match op {
        Some(o) => o,
        None => anyhow::bail!(
            "Specify a PDF operation with --op split|rasterize, or describe it with --prompt."
        ),
    };
    if op != "split" && op != "rasterize" {
        anyhow::bail!("Unknown --op '{op}'. Use 'split' or 'rasterize'.");
    }

    if op == "split" {
        return Ok(PdfParams {
            op,
            format: None,
            dpi: None,
            quality: None,
        });
    }

    let format = format.or_else(|| prompt.as_ref().and_then(|p| p.format.clone()));
    let dpi = dpi.or_else(|| prompt.as_ref().and_then(|p| p.dpi));
    let quality = quality.or_else(|| prompt.as_ref().and_then(|p| p.quality));
    Ok(PdfParams {
        op,
        format: Some(format.unwrap_or_else(|| "png".to_string())),
        dpi: Some(dpi.unwrap_or(150)),
        quality,
    })
}

fn print_pdf_summary(p: &PdfParams) {
    let desc = if p.op == "split" {
        "split into per-page PDFs".to_string()
    } else {
        let fmt = p.format.as_deref().unwrap_or("png").to_uppercase();
        let dpi = p.dpi.unwrap_or(150);
        format!("rasterize to {fmt} at {dpi} DPI")
    };
    eprintln!("Interpreted: {desc}");
}

fn format_params_summary(p: &ProcessParams) -> String {
    let mut parts = Vec::new();
    if let Some(ref fmt) = p.format {
        parts.push(fmt.clone());
    }
    match (p.width, p.height) {
        (Some(w), Some(h)) => parts.push(format!("{w} × {h}")),
        (Some(w), None) => parts.push(format!("{w}w")),
        (None, Some(h)) => parts.push(format!("{h}h")),
        _ => {}
    }
    if p.crop == Some(true) {
        parts.push("crop".into());
    }
    if p.rotation.map(|r| r != 0).unwrap_or(false) {
        parts.push(format!("rotate {}°", p.rotation.unwrap()));
    }
    if p.clarity == Some(true) {
        parts.push("clarity".into());
    }
    if p.remove_background == Some(true) {
        parts.push("remove bg".into());
    }
    if let Some(ref bg) = p.background {
        parts.push(format!("bg {bg}"));
    }
    if parts.is_empty() {
        "original settings".into()
    } else {
        parts.join(" · ")
    }
}

fn print_prompt_summary(
    files: &[PathBuf],
    map: &std::collections::HashMap<String, Vec<ProcessParams>>,
) {
    eprintln!("Interpreted:");
    for file_path in files {
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if let Some(variants) = map.get(filename) {
            if variants.len() == 1 {
                eprintln!("  {filename} → {}", format_params_summary(&variants[0]));
            } else {
                eprintln!("  {filename} →");
                for v in variants {
                    eprintln!("    {}", format_params_summary(v));
                }
            }
        }
    }
}

fn print_squish_meta(meta: &SquishMeta) {
    let mut parts = Vec::new();
    if let Some(ref ms) = meta.latency_ms {
        parts.push(format!("{ms}ms"));
    }
    if meta.optimized {
        parts.push("optimized".into());
    } else {
        parts.push("not optimized".into());
        if let Some(ref r) = meta.reason {
            parts.push(format!("({r})"));
        }
    }
    if let Some(ref q) = meta.quality {
        parts.push(format!("quality {q}"));
    }
    if let Some(ref s) = meta.saliency {
        parts.push(format!("saliency {s}"));
    }
    if meta.bg_removed {
        parts.push("bg removed".into());
    }
    eprintln!("  ← {}", parts.join(" · "));
}

/// Merge prompt-derived `base` params with explicit CLI `overrides`.
/// Any explicitly set field in `overrides` wins; unset fields fall back to `base`.
fn merge_params(base: ProcessParams, overrides: ProcessParams) -> ProcessParams {
    ProcessParams {
        format: overrides.format.or(base.format),
        width: overrides.width.or(base.width),
        height: overrides.height.or(base.height),
        crop: overrides.crop.or(base.crop),
        rotation: overrides.rotation.or(base.rotation),
        out_name_suffix: base.out_name_suffix, // always from NLP — explicit flags don't override naming
        output_name: overrides.output_name.or(base.output_name),
        clarity: overrides.clarity.or(base.clarity),
        remove_background: overrides.remove_background.or(base.remove_background),
        background: overrides.background.or(base.background),
    }
}

fn spinner(msg: impl Into<String>) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.into());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

async fn run_mcp_server(api_key: Option<String>) -> Result<()> {
    use rmcp::ServiceExt;

    let server = mcp::MochifyMcp::new(api_key)
        .serve(rmcp::transport::stdio())
        .await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{format_params_summary, merge_params, resolve_pdf_params};
    use crate::api::{PdfParams, ProcessParams};

    #[test]
    fn explicit_override_wins_unset_falls_back_to_prompt() {
        let base = ProcessParams {
            format: Some("jpg".into()),
            width: Some(800),
            ..Default::default()
        };
        let overrides = ProcessParams {
            format: Some("avif".into()),
            ..Default::default()
        };
        let merged = merge_params(base, overrides);
        assert_eq!(merged.format.as_deref(), Some("avif")); // explicit flag wins
        assert_eq!(merged.width, Some(800)); // unset → prompt value
    }

    #[test]
    fn out_name_suffix_always_comes_from_prompt() {
        let base = ProcessParams {
            out_name_suffix: Some("_500w".into()),
            ..Default::default()
        };
        let overrides = ProcessParams {
            out_name_suffix: Some("_ignored".into()),
            ..Default::default()
        };
        let merged = merge_params(base, overrides);
        assert_eq!(merged.out_name_suffix.as_deref(), Some("_500w"));
    }

    #[test]
    fn summary_lists_set_params() {
        let p = ProcessParams {
            format: Some("webp".into()),
            width: Some(1200),
            height: Some(800),
            remove_background: Some(true),
            ..Default::default()
        };
        let s = format_params_summary(&p);
        assert!(s.contains("webp"));
        assert!(s.contains("1200 × 800"));
        assert!(s.contains("remove bg"));
    }

    #[test]
    fn summary_of_empty_params_is_original_settings() {
        assert_eq!(
            format_params_summary(&ProcessParams::default()),
            "original settings"
        );
    }

    #[test]
    fn pdf_rasterize_applies_png_150_defaults() {
        let p = resolve_pdf_params(None, Some("rasterize".into()), None, None, None).unwrap();
        assert_eq!(p.op, "rasterize");
        assert_eq!(p.format.as_deref(), Some("png"));
        assert_eq!(p.dpi, Some(150));
    }

    #[test]
    fn pdf_split_drops_render_params() {
        let p = resolve_pdf_params(
            None,
            Some("split".into()),
            Some("png".into()),
            Some(300),
            None,
        )
        .unwrap();
        assert_eq!(p.op, "split");
        assert_eq!(p.format, None);
        assert_eq!(p.dpi, None);
    }

    #[test]
    fn pdf_explicit_flags_override_prompt() {
        let prompt = PdfParams {
            op: "rasterize".into(),
            format: Some("png".into()),
            dpi: Some(150),
            quality: None,
        };
        let p = resolve_pdf_params(Some(prompt), None, Some("webp".into()), Some(300), Some(80))
            .unwrap();
        assert_eq!(p.format.as_deref(), Some("webp")); // explicit flag wins
        assert_eq!(p.dpi, Some(300));
        assert_eq!(p.quality, Some(80));
    }

    #[test]
    fn pdf_prompt_seeds_op_when_no_flag() {
        let prompt = PdfParams {
            op: "split".into(),
            format: None,
            dpi: None,
            quality: None,
        };
        let p = resolve_pdf_params(Some(prompt), None, None, None, None).unwrap();
        assert_eq!(p.op, "split");
    }

    #[test]
    fn pdf_requires_op_or_prompt() {
        assert!(resolve_pdf_params(None, None, None, None, None).is_err());
    }

    #[test]
    fn pdf_rejects_unknown_op() {
        assert!(resolve_pdf_params(None, Some("flatten".into()), None, None, None).is_err());
    }
}
