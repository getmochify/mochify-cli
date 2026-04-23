use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
struct Credentials {
    api_key: String,
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("mochify").join("credentials.toml"))
}

pub fn load() -> Option<String> {
    let content = std::fs::read_to_string(path()?).ok()?;
    toml::from_str::<Credentials>(&content).ok().map(|c| c.api_key)
}

pub fn save(api_key: &str) -> Result<()> {
    let p = path().context("could not determine config directory")?;
    std::fs::create_dir_all(p.parent().unwrap()).context("failed to create config directory")?;
    let content = toml::to_string(&Credentials { api_key: api_key.to_owned() }).unwrap();
    std::fs::write(&p, content).context("failed to write credentials")
}

pub fn clear() -> Result<()> {
    if let Some(p) = path() {
        if p.exists() {
            std::fs::remove_file(&p).context("failed to remove credentials")?;
        }
    }
    Ok(())
}
