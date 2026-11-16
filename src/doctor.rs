//! `stele doctor`: what is set up, what is not, and the command that fixes it.
//!
//! It reads and probes; it never changes anything. The rows are plain text so
//! they stay readable in a journal, and every failure carries a hint, because
//! the point is to stop guessing.

use crate::client::Api;
use crate::config::{default_path, ClientConfig};
use crate::failure::{self, Kind};
use crate::mem::{at_least, Mem, MIN_VERSION};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs every check and returns how many failed. Warnings do not count: a
/// world-readable config or a stopped timer is worth saying, not failing.
pub fn run(config_path: Option<PathBuf>) -> Result<usize> {
    let path = match config_path {
        Some(path) => path,
        None => default_path()?,
    };
    let mut failures = 0;
    let config = check_config(&path, &mut failures);
    let mem = Mem::new(
        config.as_ref().and_then(|config| config.mem_bin.clone()),
        config.as_ref().and_then(|config| config.mem_db.clone()),
    );
    if let Some(config) = &config {
        check_server(config, &mut failures);
    }
    if check_mem(&mem, &mut failures) {
        check_drafts(&mem, &mut failures);
    }
    check_timer();
    Ok(failures)
}

fn row(name: &str, level: &str, detail: &str) {
    println!("{name:<8} {level:<4} {detail}");
}

fn hint(text: &str) {
    println!("{:<13} {text}", "");
}

fn check_config(path: &Path, failures: &mut usize) -> Option<ClientConfig> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => {
            row(
                "config",
                "fail",
                &format!("{} is missing or unreadable", path.display()),
            );
            hint("run `stele login --url URL --token -`");
            *failures += 1;
            return None;
        }
    };
    let config: ClientConfig = match serde_json::from_str(&raw) {
        Ok(config) => config,
        Err(error) => {
            row(
                "config",
                "fail",
                &format!("{} is not valid JSON: {error}", path.display()),
            );
            hint("run `stele login` again to rewrite it");
            *failures += 1;
            return None;
        }
    };
    match mode_of(path) {
        Some(mode) if mode & 0o077 != 0 => {
            row(
                "config",
                "warn",
                &format!(
                    "{} is readable by others (mode {:o})",
                    path.display(),
                    mode & 0o777
                ),
            );
            hint(&format!("chmod 600 {}", path.display()));
        }
        Some(_) => row("config", "ok", &format!("{} (0600)", path.display())),
        None => row("config", "ok", &format!("{}", path.display())),
    }
    Some(config)
}

fn check_server(config: &ClientConfig, failures: &mut usize) {
    let api = match Api::new(&config.url, config.token.clone()) {
        Ok(api) => api,
        Err(error) => {
            row("server", "fail", &format!("{error:#}"));
            hint("fix the url in the config, or run `stele login`");
            *failures += 1;
            return;
        }
    };
    match api.status() {
        Ok(status) => row(
            "server",
            "ok",
            &format!(
                "{}, token valid for {}",
                api.endpoint.authority, status.mailbox
            ),
        ),
        Err(error) => {
            row("server", "fail", &format!("{error:#}"));
            match failure::kind(&error) {
                Some(Kind::Unreachable) => {
                    hint("check the address and that `stele serve` runs on the other machine")
                }
                Some(Kind::Refused) => hint("the token is not accepted; run `stele login` again"),
                None => hint("see the message above"),
            }
            *failures += 1;
        }
    }
}

/// Runs the binary and its version. Returns whether the rest of the anathem
/// checks make sense: there is no point asking for drafts from a `mem` that
/// is too old, and the second failure would only bury the first.
fn check_mem(mem: &Mem, failures: &mut usize) -> bool {
    match mem.version() {
        Err(error) => {
            row("mem", "fail", &format!("{error:#}"));
            hint("install anathem 1.2 or newer, or set mem_bin in the config");
            *failures += 1;
            false
        }
        Ok(line) if at_least(&line, MIN_VERSION) => {
            row("mem", "ok", &line);
            true
        }
        Ok(line) => {
            row("mem", "fail", &line);
            hint(&format!("stele needs anathem {MIN_VERSION} or newer"));
            *failures += 1;
            false
        }
    }
}

fn check_drafts(mem: &Mem, failures: &mut usize) {
    match mem.drafts_waiting() {
        Ok(count) => row("drafts", "ok", &format!("{count} waiting")),
        Err(error) => {
            row("drafts", "fail", &format!("{error:#}"));
            hint("check mem_db, or let anathem create its database with `mem init`");
            *failures += 1;
        }
    }
}

/// Best effort: the unit belongs to a deployment, not to the protocol. A
/// machine without systemd, or one that pulls from cron, is not broken.
fn check_timer() {
    let present = Command::new("systemctl")
        .args(["--user", "cat", "stele-pull.timer"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !present {
        row(
            "timer",
            "skip",
            "no stele-pull.timer here (cron or a manual pull is fine too)",
        );
        return;
    }
    let active = Command::new("systemctl")
        .args(["--user", "is-active", "stele-pull.timer"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if active {
        row("timer", "ok", "stele-pull.timer active");
    } else {
        row(
            "timer",
            "warn",
            "stele-pull.timer is not running; mail waits until somebody pulls",
        );
        hint("systemctl --user start stele-pull.timer");
    }
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode())
}

#[cfg(not(unix))]
fn mode_of(_path: &Path) -> Option<u32> {
    None
}
