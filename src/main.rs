mod api;
mod cli;
mod credentials;
mod mcp;

use anyhow::{Context, Result};
use api::{MochifyClient, PdfMeta, PdfOptions, PdfParams, PdfPrompt, ProcessParams, SquishMeta};
use clap::Parser;
use cli::{Args, AuthAction, Commands};
use indicatif::{ProgressBar, ProgressStyle};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use tokio::task::JoinSet;

const WORKER_URL: &str = "https://id.mochify.app";
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
            if !usage.plan.is_empty() {
                println!("Plan: {}", usage.plan);
            }
            if usage.quota > 0 {
                println!("Remaining: {} / {}", usage.remaining, usage.quota);
            } else {
                println!("Remaining: {}", usage.remaining);
            }
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

    // `--op create` is the one PDF op whose inputs are images, so it is chosen by the
    // flag rather than by extension — there is nothing in a .jpg to route on.
    if args
        .op
        .as_deref()
        .map(|o| o.trim().eq_ignore_ascii_case("create"))
        .unwrap_or(false)
    {
        if has_pdf {
            anyhow::bail!(
                "--op create builds a PDF from images — pass images, not PDFs. \
                 To make an existing PDF smaller, use --op optimize."
            );
        }
        return create_pdf(&args, &client).await;
    }

    if has_pdf {
        if args.files.iter().any(|p| !is_pdf(p)) {
            anyhow::bail!("Can't mix PDFs and images in one command — run them separately.");
        }
        return process_pdfs(&args, &client).await;
    }

    // Reject rotations the API won't accept before doing any work.
    if let Some(r) = args.rotation
        && !matches!(r, 0 | 90 | 180 | 270)
    {
        anyhow::bail!("Invalid --rotation {r}. Use 0, 90, 180, or 270.");
    }

    // Fail on a bad --hdr mode before uploading anything.
    let hdr = match args.hdr.as_deref() {
        Some(mode) => Some(api::normalize_hdr_mode(mode)?),
        None => None,
    };

    if let Some(q) = args.quality
        && !(1..=100).contains(&q)
    {
        anyhow::bail!("Invalid --quality {q}. Use 1–100.");
    }
    if let Some(b) = args.brightness
        && !(-100..=100).contains(&b)
    {
        anyhow::bail!("Invalid --brightness {b}. Use -100 (darkest) to 100 (brightest).");
    }
    // The API answers lossless + jpg/avif with a 400, so catch it here rather than
    // spending a request to be told.
    if args.lossless
        && let Some(ref fmt) = args.format
    {
        let fmt = fmt.trim().to_lowercase();
        let fmt = if fmt == "jpeg" {
            "jpg".to_string()
        } else {
            fmt
        };
        if !api::LOSSLESS_FORMATS.contains(&fmt.as_str()) {
            anyhow::bail!(
                "--lossless can't be used with -t {fmt}. Only {} can hold pixel-exact output.",
                api::LOSSLESS_FORMATS.join(", ")
            );
        }
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
        background: args.background,
        strip_exif: if args.keep_metadata {
            Some(false)
        } else {
            None
        },
        hdr,
        quality: args.quality,
        smart_compress: args.smart_compress.then_some(true),
        brightness: args.brightness,
        optimize_for_web: args.optimize_for_web.then_some(true),
        lossless: args.lossless.then_some(true),
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

    // Build the whole job list before running any of it. One job per (file, variant):
    // variants are independent requests, so a single file the prompt answers with two
    // formats overlaps exactly the way two files do.
    let mut jobs: Vec<(String, SquishJob)> = Vec::new();

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
        for params in variants {
            let label = match &params.out_name_suffix {
                Some(s) => format!("{name}{s}"),
                None => name.clone(),
            };
            let client = client.clone();
            let file_path = file_path.clone();
            let out_dir = out_dir.clone();
            jobs.push((
                label,
                Box::pin(async move {
                    let (out, meta) = client.squish(&file_path, &params, &out_dir).await?;
                    // The params travel back out with the result because the warnings
                    // below are about what was *asked* for versus what came back.
                    Ok((out, meta, params))
                }),
            ));
        }
    }

    let verbose = args.verbose;
    run_batch(jobs, args.jobs, |label, result| match result {
        Ok((out, meta, params)) => {
            println!("{}", out.display());
            warn_if_hdr_dropped(&params, &meta);
            warn_if_lossless_downgraded(&params, &meta);
            if verbose {
                print_squish_meta(&meta);
            }
        }
        Err(e) => eprintln!("Error processing {label}: {e:#}"),
    })
    .await;

    Ok(())
}

/// One unit of work for [`run_batch`]. Boxed because the jobs are built in a loop and
/// handed to `tokio::spawn`, which needs a single concrete owned type.
type Job<R> = Pin<Box<dyn Future<Output = Result<R>> + Send>>;

/// One squish. The params come back out alongside the result because the warnings the
/// reporter prints compare what was asked for against what the API returned.
type SquishJob = Job<(PathBuf, SquishMeta, ProcessParams)>;

/// Run `jobs` with at most `limit` in flight, reporting each one in the order it was
/// submitted rather than the order it finished.
///
/// Both halves matter. Before this, every file was one awaited round trip after
/// another, so a batch spent nearly all its wall time with an idle connection — but a
/// user who passed `a.jpg b.jpg c.jpg` (or piped `find` into us) still expects the
/// printed paths in that order, and so does anything reading our stdout. So the work
/// overlaps and the output does not: a finished job waits in its slot until every job
/// before it has been reported.
async fn run_batch<R: Send + 'static>(
    jobs: Vec<(String, Job<R>)>,
    limit: usize,
    report: impl Fn(&str, Result<R>),
) {
    let total = jobs.len();
    if total == 0 {
        return;
    }

    let (labels, futures): (Vec<String>, Vec<Job<R>>) = jobs.into_iter().unzip();

    // A single file keeps the old message — "Processing photo.jpg..." says more than
    // "Processing 0/1..." does.
    let pb = spinner(if total == 1 {
        format!("Processing {}...", labels[0])
    } else {
        format!("Processing 0/{total}...")
    });

    let mut queued = futures.into_iter().enumerate();
    let mut set: JoinSet<(usize, Result<R>)> = JoinSet::new();
    let mut slots: Vec<Option<Result<R>>> = (0..total).map(|_| None).collect();
    let mut cursor = 0;
    let mut finished = 0;

    let mut spawn_next = |set: &mut JoinSet<(usize, Result<R>)>| {
        if let Some((i, fut)) = queued.next() {
            set.spawn(async move { (i, fut.await) });
        }
    };

    for _ in 0..limit.max(1) {
        spawn_next(&mut set);
    }

    while let Some(joined) = set.join_next().await {
        // A JoinError means the task panicked, which leaves its slot empty. Don't
        // report it here: the slot's position in the output order still has to be
        // honoured, so the final pass below handles it.
        if let Ok((i, result)) = joined {
            slots[i] = Some(result);
        }
        finished += 1;
        if total > 1 {
            pb.set_message(format!("Processing {finished}/{total}..."));
        }
        // Drain every slot that is now contiguous with what has already been printed.
        while cursor < total && slots[cursor].is_some() {
            let result = slots[cursor].take().unwrap();
            pb.suspend(|| report(&labels[cursor], result));
            cursor += 1;
        }
        spawn_next(&mut set);
    }

    pb.finish_and_clear();

    // Anything still unreported was blocked behind a panicked job.
    for (i, label) in labels.iter().enumerate().skip(cursor) {
        match slots[i].take() {
            Some(result) => report(label, result),
            None => report(
                label,
                Err(anyhow::anyhow!("worker task failed unexpectedly")),
            ),
        }
    }
}

async fn process_pdfs(args: &Args, client: &MochifyClient) -> Result<()> {
    // Resolve the operation: a prompt (if given) seeds it via NLP, then explicit
    // flags override. Mirrors the image flow's prompt-then-flags precedence.
    let prompt_params = resolve_prompt_for_pdf(args, client, "pdf").await?;

    let params = resolve_pdf_params(prompt_params, args)?;
    // create takes images in, so it can't run on the PDF path — reachable only if the
    // NLP answers with it for a PDF input.
    if params.op == "create" {
        anyhow::bail!(
            "--op create builds a PDF from images. To make this PDF smaller, use --op optimize."
        );
    }
    print_pdf_summary(&params, args.files.len());

    let mut jobs: Vec<(String, Job<(PathBuf, PdfMeta)>)> = Vec::new();

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
        let client = client.clone();
        let file_path = file_path.clone();
        // Every PDF in the invocation runs the same op with the same params, so unlike
        // the image path there is nothing per-job to carry back out.
        let params = params.clone();
        jobs.push((
            name,
            Box::pin(async move { client.pdf(&file_path, &params, &out_dir).await }),
        ));
    }

    let verbose = args.verbose;
    run_batch(jobs, args.jobs, |label, result| match result {
        Ok((out, meta)) => {
            println!("{}", out.display());
            print_pdf_meta(&params, &meta, verbose);
        }
        Err(e) => eprintln!("Error processing {label}: {e:#}"),
    })
    .await;

    Ok(())
}

/// `--op create`: build a PDF from images. Every file goes up in one multipart request
/// (one page per image, in the order given), so this is a single call, not a loop, and
/// a single output — one combined PDF, or a zip of one-page PDFs with `--no-combine`.
async fn create_pdf(args: &Args, client: &MochifyClient) -> Result<()> {
    let prompt_params = resolve_prompt_for_pdf(args, client, "imgpdf").await?;
    let params = resolve_pdf_params(prompt_params, args)?;
    print_pdf_summary(&params, args.files.len());

    let out_dir = match &args.output {
        Some(d) => d.clone(),
        None => args
            .files
            .first()
            .and_then(|f| f.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
    };

    let count = args.files.len();
    let sp = spinner(format!(
        "Building PDF from {count} image{}...",
        if count == 1 { "" } else { "s" }
    ));
    match client
        .pdf_create(&args.files, &params, &out_dir, args.name.as_deref())
        .await
    {
        Ok((out, meta)) => {
            sp.finish_and_clear();
            println!("{}", out.display());
            print_pdf_meta(&params, &meta, args.verbose);
            Ok(())
        }
        Err(e) => {
            sp.finish_and_clear();
            anyhow::bail!("{e:#}");
        }
    }
}

/// Run the NLP prompt for a PDF flow, if one was given. `mode` is "pdf" (PDF in) or
/// "imgpdf" (images in, PDF out) — they are separate schemas on the worker.
async fn resolve_prompt_for_pdf(
    args: &Args,
    client: &MochifyClient,
    mode: &str,
) -> Result<Option<PdfPrompt>> {
    let Some(ref prompt) = args.prompt else {
        return Ok(None);
    };
    let sp = spinner("Parsing prompt...");
    let paths: Vec<&std::path::Path> = args.files.iter().map(|p| p.as_path()).collect();
    let (params, raw_json) = client.resolve_pdf_prompt(prompt, &paths, mode).await?;
    sp.finish_and_clear();
    if args.verbose {
        eprintln!("Prompt response JSON:");
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&raw_json).unwrap_or_default()
        );
    }
    Ok(Some(params))
}

/// Combine prompt-derived and explicit PDF params — an explicit flag always wins — then
/// hand them to `PdfParams::for_op`, which validates the op and decides which of them
/// this op actually sends.
fn resolve_pdf_params(prompt: Option<PdfPrompt>, args: &Args) -> Result<PdfParams> {
    let op = args
        .op
        .clone()
        .or_else(|| prompt.as_ref().map(|p| p.op.clone()));
    let Some(op) = op else {
        anyhow::bail!(
            "Specify a PDF operation with --op optimize|extract|rasterize|split, \
             or describe it with --prompt."
        )
    };

    let opts = PdfOptions {
        format: args
            .format
            .clone()
            .or_else(|| prompt.as_ref().and_then(|p| p.format.clone())),
        dpi: args.dpi.or_else(|| prompt.as_ref().and_then(|p| p.dpi)),
        quality: args
            .quality
            .or_else(|| prompt.as_ref().and_then(|p| p.quality)),
        max_width: args
            .max_width
            .or_else(|| prompt.as_ref().and_then(|p| p.max_width)),
        min_size: args.min_size,
        page: args
            .page
            .clone()
            .or_else(|| prompt.as_ref().and_then(|p| p.page.clone())),
        // --no-combine is a flag, so it can only ever mean "separate PDFs"; unset leaves
        // the prompt's answer (or the API default, one combined document) in place.
        combine: if args.no_combine {
            Some(false)
        } else {
            prompt.as_ref().and_then(|p| p.combine)
        },
    };

    PdfParams::for_op(&op, opts)
}

fn print_pdf_summary(p: &PdfParams, files: usize) {
    let quality = |suffix: &str| match p.quality {
        Some(q) => format!("{suffix} at quality {q}"),
        None => suffix.to_string(),
    };
    let desc = match p.op.as_str() {
        "split" => "split into per-page PDFs".to_string(),
        "extract" => {
            let fmt = match p.format.as_deref() {
                None | Some("original") => "in their original encoding".to_string(),
                Some(f) => format!("as {}", f.to_uppercase()),
            };
            let mut d = format!("extract the embedded images {fmt}");
            if let Some(w) = p.max_width.filter(|&w| w > 0) {
                d.push_str(&format!(", capped at {w}px wide"));
            }
            d
        }
        "optimize" => {
            let mut d = quality("recompress the images inside the PDF");
            if let Some(dpi) = p.max_dpi {
                d.push_str(&format!(", max {dpi} DPI"));
            }
            if let Some(px) = p.max_dimension.filter(|&px| px > 0) {
                d.push_str(&format!(", max {px}px"));
            }
            d
        }
        "create" => {
            let page = p.page.as_deref().unwrap_or("fit");
            let pages = if page == "fit" {
                "pages sized to each image".to_string()
            } else {
                format!("{} pages", page.to_uppercase())
            };
            if p.combine == Some(false) {
                format!("build one PDF per image from {files} images, {pages}")
            } else {
                format!("build one PDF from {files} images, {pages}")
            }
        }
        _ => {
            let fmt = p.format.as_deref().unwrap_or("png").to_uppercase();
            let dpi = p.dpi.unwrap_or(150);
            format!("rasterize to {fmt} at {dpi} DPI")
        }
    };
    eprintln!("Interpreted: {desc}");
}

/// Report what /v1/pdf said it did. `optimize`'s saving is the whole point of the op,
/// so it is always shown; the rest of the headers are verbose-only detail.
fn print_pdf_meta(params: &PdfParams, meta: &PdfMeta, verbose: bool) {
    if params.op == "optimize"
        && let Some(ref pct) = meta.saved_pct
    {
        // 0% is not a failure: the API returns the original bytes when recompressing
        // them would not have helped.
        if pct == "0" {
            eprintln!("  ← already well optimized — the original was returned unchanged");
        } else {
            eprintln!("  ← {pct}% smaller");
        }
    }
    if !verbose {
        return;
    }
    let mut parts = Vec::new();
    if let Some(ref ms) = meta.latency_ms {
        parts.push(format!("{ms}ms"));
    }
    if let Some(ref pages) = meta.pages {
        parts.push(format!("{pages} pages"));
    }
    if let Some(ref images) = meta.images {
        parts.push(format!("{images} images"));
    }
    if let Some(ref n) = meta.recompressed {
        parts.push(format!("{n} recompressed"));
    }
    if !parts.is_empty() {
        eprintln!("  ← {}", parts.join(" · "));
    }
}

/// `X-Mochify-HDR: false` on a request that asked for HDR means the bytes carry no gain
/// map. The file is otherwise fine, so this is a note rather than an error — but without
/// it the most common cause (a format that cannot hold one) is invisible.
/// `X-Mochify-Lossless: downgraded` means the bytes are the best lossy encode rather
/// than pixel-exact — nothing can restore what an already-lossy source discarded. Worth
/// saying, since the request asked for something it did not get.
fn warn_if_lossless_downgraded(params: &ProcessParams, meta: &SquishMeta) {
    if params.lossless == Some(true) && meta.lossless.as_deref() == Some("downgraded") {
        eprintln!(
            "  note: the source was already lossy, so the output is the best lossy encode rather than pixel-exact."
        );
    }
}

fn warn_if_hdr_dropped(params: &ProcessParams, meta: &SquishMeta) {
    if params.hdr.is_none() || meta.hdr.as_deref() != Some("false") {
        return;
    }
    let carries_hdr = matches!(params.format.as_deref(), None | Some("jpg") | Some("jpeg"));
    if !carries_hdr {
        eprintln!(
            "  note: no HDR gain map in the output — only jpg output can carry one. Re-run with -t jpg."
        );
    } else if params.hdr.as_deref() == Some("1") {
        eprintln!(
            "  note: the source had no gain map to preserve. Use --hdr generate to synthesise one."
        );
    } else {
        eprintln!("  note: the output carries no HDR gain map.");
    }
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
    if p.strip_exif == Some(false) {
        parts.push("keep metadata".into());
    }
    if let Some(ref hdr) = p.hdr {
        parts.push(if hdr == "generate" {
            "hdr (generate)".into()
        } else {
            "hdr (preserve)".into()
        });
    }
    if let Some(q) = p.quality {
        parts.push(format!("quality {q}"));
    }
    if p.smart_compress == Some(true) {
        parts.push("smart compress".into());
    }
    if let Some(b) = p.brightness.filter(|&b| b != 0) {
        parts.push(format!("brightness {b:+}"));
    }
    if p.optimize_for_web == Some(true) {
        parts.push("optimize for web".into());
    }
    if p.lossless == Some(true) {
        parts.push("lossless".into());
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
    if let Some(ref hdr) = meta.hdr {
        parts.push(format!("hdr {hdr}"));
    }
    if let Some(ref lossless) = meta.lossless {
        parts.push(format!("lossless {lossless}"));
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
        strip_exif: overrides.strip_exif.or(base.strip_exif),
        hdr: overrides.hdr.or(base.hdr),
        quality: overrides.quality.or(base.quality),
        smart_compress: overrides.smart_compress.or(base.smart_compress),
        brightness: overrides.brightness.or(base.brightness),
        optimize_for_web: overrides.optimize_for_web.or(base.optimize_for_web),
        lossless: overrides.lossless.or(base.lossless),
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
    use super::{
        Job, format_params_summary, merge_params, resolve_pdf_params, run_batch,
        warn_if_hdr_dropped,
    };
    use crate::api::{PdfPrompt, ProcessParams, SquishMeta};
    use crate::cli::Args;
    use clap::Parser;
    use std::cell::RefCell;
    use std::time::Duration;

    /// Parse flags the way the binary does, so the tests exercise the real clap config.
    fn args(flags: &[&str]) -> Args {
        let mut argv = vec!["mochify", "doc.pdf"];
        argv.extend_from_slice(flags);
        Args::parse_from(argv)
    }

    fn pdf_prompt(op: &str) -> PdfPrompt {
        PdfPrompt {
            op: op.into(),
            format: Some("png".into()),
            dpi: Some(150),
            quality: Some(85),
            max_width: Some(0),
            page: None,
            combine: None,
        }
    }

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
        let p = resolve_pdf_params(None, &args(&["--op", "rasterize"])).unwrap();
        assert_eq!(p.op, "rasterize");
        assert_eq!(p.format.as_deref(), Some("png"));
        assert_eq!(p.dpi, Some(150));
    }

    #[test]
    fn pdf_split_drops_render_params() {
        let p = resolve_pdf_params(None, &args(&["--op", "split", "-t", "png", "--dpi", "300"]))
            .unwrap();
        assert_eq!(p.op, "split");
        assert_eq!(p.format, None);
        assert_eq!(p.dpi, None);
    }

    #[test]
    fn pdf_explicit_flags_override_prompt() {
        let p = resolve_pdf_params(
            Some(pdf_prompt("rasterize")),
            &args(&["-t", "webp", "--dpi", "300", "-q", "80"]),
        )
        .unwrap();
        assert_eq!(p.op, "rasterize"); // op came from the prompt
        assert_eq!(p.format.as_deref(), Some("webp")); // explicit flag wins
        assert_eq!(p.dpi, Some(300));
        assert_eq!(p.quality, Some(80));
    }

    #[test]
    fn pdf_prompt_seeds_op_when_no_flag() {
        let p = resolve_pdf_params(Some(pdf_prompt("split")), &args(&[])).unwrap();
        assert_eq!(p.op, "split");
    }

    #[test]
    fn pdf_requires_op_or_prompt() {
        assert!(resolve_pdf_params(None, &args(&[])).is_err());
    }

    #[test]
    fn pdf_rejects_unknown_op() {
        assert!(resolve_pdf_params(None, &args(&["--op", "flatten"])).is_err());
    }

    #[test]
    fn pdf_optimize_maps_dpi_and_width_to_image_caps() {
        let p = resolve_pdf_params(
            None,
            &args(&[
                "--op",
                "optimize",
                "--dpi",
                "96",
                "--max-width",
                "1200",
                "-q",
                "70",
            ]),
        )
        .unwrap();
        assert_eq!(p.max_dpi, Some(96)); // --dpi means maxDpi here
        assert_eq!(p.max_dimension, Some(1200)); // --max-width means maxDimension here
        assert_eq!(p.dpi, None);
        assert_eq!(p.quality, Some(70));
    }

    #[test]
    fn pdf_extract_keeps_original_and_width_cap() {
        let p = resolve_pdf_params(
            None,
            &args(&["--op", "extract", "-t", "original", "--max-width", "1600"]),
        )
        .unwrap();
        assert_eq!(p.format.as_deref(), Some("original"));
        assert_eq!(p.max_width, Some(1600));
    }

    #[test]
    fn pdf_rasterize_rejects_original_which_only_extract_takes() {
        assert!(resolve_pdf_params(None, &args(&["--op", "rasterize", "-t", "original"])).is_err());
    }

    #[test]
    fn pdf_format_aliases_jpeg_to_jpg() {
        let p = resolve_pdf_params(None, &args(&["--op", "rasterize", "-t", "JPEG"])).unwrap();
        assert_eq!(p.format.as_deref(), Some("jpg"));
    }

    #[test]
    fn pdf_create_takes_page_and_no_combine() {
        let p = resolve_pdf_params(
            None,
            &args(&["--op", "create", "--page", "A4", "--no-combine"]),
        )
        .unwrap();
        assert_eq!(p.op, "create");
        assert_eq!(p.page.as_deref(), Some("a4"));
        assert_eq!(p.combine, Some(false));
    }

    #[test]
    fn pdf_create_rejects_unknown_page_size() {
        assert!(resolve_pdf_params(None, &args(&["--op", "create", "--page", "a3"])).is_err());
    }

    #[test]
    fn bare_hdr_flag_means_preserve() {
        // --hdr with no value, and --hdr generate, are the two ways in.
        assert_eq!(args(&["--hdr"]).hdr.as_deref(), Some("preserve"));
        assert_eq!(
            args(&["--hdr", "generate"]).hdr.as_deref(),
            Some("generate")
        );
    }

    #[test]
    fn quality_flags_summarise() {
        let s = format_params_summary(&ProcessParams {
            quality: Some(70),
            smart_compress: Some(true),
            brightness: Some(-20),
            optimize_for_web: Some(true),
            lossless: Some(true),
            ..Default::default()
        });
        assert!(s.contains("quality 70"));
        assert!(s.contains("smart compress"));
        assert!(s.contains("brightness -20"));
        assert!(s.contains("optimize for web"));
        assert!(s.contains("lossless"));
    }

    #[test]
    fn negative_brightness_parses_as_a_value_not_a_flag() {
        let a = Args::parse_from(["mochify", "photo.jpg", "--brightness", "-40"]);
        assert_eq!(a.brightness, Some(-40));
    }

    #[test]
    fn lossless_is_a_flag_and_optimise_spelling_is_accepted() {
        let a = Args::parse_from(["mochify", "photo.jpg", "--lossless", "--optimise-for-web"]);
        assert!(a.lossless);
        assert!(a.optimize_for_web);
    }

    #[test]
    fn hdr_summary_names_the_mode() {
        let s = format_params_summary(&ProcessParams {
            hdr: Some("generate".into()),
            ..Default::default()
        });
        assert!(s.contains("hdr (generate)"));
    }

    #[test]
    fn hdr_note_only_fires_when_the_output_carries_none() {
        // Nothing requested → nothing to say, whatever the header holds.
        let quiet = ProcessParams::default();
        warn_if_hdr_dropped(
            &quiet,
            &SquishMeta {
                hdr: Some("false".into()),
                ..Default::default()
            },
        );
        // Requested and delivered → also nothing to say.
        let asked = ProcessParams {
            hdr: Some("generate".into()),
            format: Some("jpg".into()),
            ..Default::default()
        };
        warn_if_hdr_dropped(
            &asked,
            &SquishMeta {
                hdr: Some("generated".into()),
                ..Default::default()
            },
        );
    }

    /// Jobs that finish in the reverse of the order they were submitted, so the test
    /// fails if reporting ever follows completion order instead.
    fn reversed_jobs(n: u64) -> Vec<(String, Job<u64>)> {
        (0..n)
            .map(|i| {
                let label = format!("job{i}");
                let job: Job<u64> = Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis((n - i) * 20)).await;
                    Ok(i)
                });
                (label, job)
            })
            .collect()
    }

    #[tokio::test]
    async fn batch_reports_in_submission_order_not_completion_order() {
        let seen = RefCell::new(Vec::new());
        run_batch(reversed_jobs(5), 5, |label, result| {
            seen.borrow_mut().push((label.to_string(), result.unwrap()));
        })
        .await;

        let seen = seen.into_inner();
        assert_eq!(
            seen,
            (0..5u64)
                .map(|i| (format!("job{i}"), i))
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn batch_of_one_at_a_time_still_runs_everything() {
        let seen = RefCell::new(Vec::new());
        run_batch(reversed_jobs(3), 1, |_, result| {
            seen.borrow_mut().push(result.unwrap());
        })
        .await;
        assert_eq!(seen.into_inner(), vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn batch_reports_a_failed_job_and_keeps_going() {
        let jobs: Vec<(String, Job<u64>)> = vec![
            ("ok".into(), Box::pin(async { Ok(1) })),
            (
                "bad".into(),
                Box::pin(async { anyhow::bail!("upload refused") }),
            ),
            ("also-ok".into(), Box::pin(async { Ok(3) })),
        ];

        let seen = RefCell::new(Vec::new());
        run_batch(jobs, 4, |label, result| {
            seen.borrow_mut().push((label.to_string(), result.is_ok()));
        })
        .await;

        assert_eq!(
            seen.into_inner(),
            vec![
                ("ok".to_string(), true),
                ("bad".to_string(), false),
                ("also-ok".to_string(), true),
            ]
        );
    }
}
