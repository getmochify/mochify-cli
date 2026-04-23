mod api;
mod cli;
mod credentials;
mod mcp;

use anyhow::Result;
use api::{MochifyClient, ProcessParams};
use clap::Parser;
use cli::{Args, AuthAction, Commands};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::time::Duration;

const WORKER_URL: &str = "https://tokens.mochify.xyz";
const AUTH_URL: &str = "https://mochify.xyz/auth/cli";

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
                if let Ok(body) = response.json::<PollResponse>().await {
                    sp.finish_and_clear();
                    credentials::save(&body.api_key)?;
                    println!("Authenticated! Credentials saved to ~/.config/mochify/credentials.toml");
                }
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
