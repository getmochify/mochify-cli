mod api;
mod cli;
mod mcp;

use anyhow::Result;
use api::{MochifyClient, ProcessParams};
use clap::Parser;
use cli::{Args, Commands};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Some(Commands::Serve) => run_mcp_server(args.api_key).await,
        None => {
            if args.files.is_empty() {
                eprintln!("No input files specified. Run with --help for usage.");
                std::process::exit(1);
            }
            process_files(args).await
        }
    }
}

async fn process_files(args: Args) -> Result<()> {
    let client = MochifyClient::new(args.api_key.clone());

    // Explicit CLI flags — these always win over prompt-derived params.
    let explicit = ProcessParams {
        format: args.format,
        width: args.width,
        height: args.height,
        crop: if args.crop { Some(true) } else { None },
        rotation: args.rotation,
    };

    // If a prompt was supplied, resolve params for all files in one request.
    let prompt_map = if let Some(ref prompt) = args.prompt {
        let sp = spinner("Parsing prompt...");
        let paths: Vec<&std::path::Path> = args.files.iter().map(|p| p.as_path()).collect();
        let map = client.resolve_prompt(prompt, &paths).await?;
        sp.finish_and_clear();
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

        let params = match &prompt_map {
            Some(map) => {
                let filename = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let base = map.get(filename).cloned().unwrap_or_default();
                merge_params(base, explicit.clone())
            }
            None => explicit.clone(),
        };

        let name = file_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let sp = spinner(format!("Uploading {name}..."));
        match client.squish(file_path, &params, &out_dir).await {
            Ok(out) => {
                sp.finish_and_clear();
                println!("{}", out.display());
            }
            Err(e) => {
                sp.finish_and_clear();
                eprintln!("Error processing {}: {e:#}", file_path.display());
            }
        }
    }

    Ok(())
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
