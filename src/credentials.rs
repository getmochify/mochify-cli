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
    toml::from_str::<Credentials>(&content)
        .ok()
        .map(|c| c.api_key)
}

pub fn save(api_key: &str) -> Result<()> {
    let p = path().context("could not determine config directory")?;
    std::fs::create_dir_all(p.parent().unwrap()).context("failed to create config directory")?;
    let content = toml::to_string(&Credentials {
        api_key: api_key.to_owned(),
    })
    .unwrap();
    write_private(&p, content.as_bytes()).context("failed to write credentials")
}

// Owner-only: the file holds an API key, and the default mode would let any
// other account on the machine read it.
#[cfg(unix)]
fn write_private(p: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(p)?;
    // mode() only applies on creation; tighten a file an older version wrote.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(p: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(p, bytes)
}

pub fn clear() -> Result<()> {
    if let Some(p) = path().filter(|p| p.exists()) {
        std::fs::remove_file(&p).context("failed to remove credentials")?;
    }
    Ok(())
}
