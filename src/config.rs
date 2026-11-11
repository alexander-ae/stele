//! The client side of the configuration: where the mailbox lives and who this
//! machine is. The token is a secret, so the file is written `0600` and a file
//! that others can read is reported instead of being used silently.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub url: String,
    pub mailbox: String,
    pub token: String,
    /// Optional: where `mem` lives, when it is not on `$PATH`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_bin: Option<String>,
    /// Optional: the anathem database `pull` should write into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_db: Option<String>,
    /// Optional shell command run after `pull` writes at least one draft. It
    /// gets `STELE_DRAFTS` and `STELE_MAILBOX`, never the messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_cmd: Option<String>,
}

impl ClientConfig {
    pub fn new(url: String, mailbox: String, token: String) -> Self {
        Self {
            url,
            mailbox,
            token,
            mem_bin: None,
            mem_db: None,
            notify_cmd: None,
        }
    }
}

/// `$STELE_CONFIG`, then `$XDG_CONFIG_HOME/stele/config.json`, then
/// `~/.config/stele/config.json`.
pub fn default_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("STELE_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("stele").join("config.json"));
        }
    }
    let home = std::env::var_os("HOME").context("HOME is not set; pass --config")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("stele")
        .join("config.json"))
}

pub fn load(path: &Path) -> Result<ClientConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("cannot read {}; run `stele login` first", path.display()))?;
    let config: ClientConfig = serde_json::from_str(&raw)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    warn_if_readable_by_others(path);
    Ok(config)
}

pub fn save(path: &Path, config: &ClientConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(config)? + "\n";
    fs::write(path, body).with_context(|| format!("cannot write {}", path.display()))?;
    set_private(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_private(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn warn_if_readable_by_others(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            eprintln!(
                "warning: {} is readable by other users (mode {:o}); the token in it is a secret",
                path.display(),
                mode & 0o777
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_readable_by_others(_path: &Path) {}

/// Reject a config that would send the token somewhere it cannot go.
pub fn validate_url(url: &str) -> Result<()> {
    if url.starts_with("https://") {
        bail!(
            "stele speaks plain HTTP: put the mailbox on a private network (a VPN, a \
             tailnet or a LAN), or wrap the port in a tunnel"
        );
    }
    if !url.starts_with("http://") {
        bail!("the url has to start with http:// (got {url:?})");
    }
    Ok(())
}
