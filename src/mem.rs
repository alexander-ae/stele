//! The anathem side of the mailbox: save one draft, and count what is
//! waiting. Both go through `mem --json` (anathem 1.2 or newer), because
//! prose is for people and a mailbox needs to tell a new draft from a retry.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// `save --external --json` and `recent --json` with `status` need 1.2.
pub const MIN_VERSION: &str = "1.2.0";

const SAVE_TIMEOUT: Duration = Duration::from_secs(10);
const RECENT_TIMEOUT: Duration = Duration::from_secs(3);
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Mem {
    bin: String,
    db: Option<String>,
}

impl Mem {
    pub fn new(bin: Option<String>, db: Option<String>) -> Self {
        Self {
            bin: bin.unwrap_or_else(|| "mem".to_string()),
            db,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.bin);
        if let Some(db) = &self.db {
            command.arg("--db").arg(db);
        }
        command
    }

    /// Writes one draft and says which of the two successes happened:
    /// `written` (a new draft) or `already_received` (this `--ext-id` was
    /// already here). Both exit 0; both are acknowledged by the caller.
    pub fn save_external(
        &self,
        body: &str,
        from: &str,
        project: &str,
        ext_id: &str,
    ) -> Result<Saved> {
        let mut command = self.command();
        command.args([
            "save",
            "-",
            "--external",
            "--from",
            from,
            "--project",
            project,
            "--ext-id",
            ext_id,
            "--json",
        ]);
        let output = run(command, Some(body), SAVE_TIMEOUT)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let hint = if old_mem(&stderr) {
                format!(" (stele needs anathem {MIN_VERSION} or newer)")
            } else {
                String::new()
            };
            bail!("mem save failed{}: {}", hint, stderr.trim());
        }
        let document = last_json(&output.stdout)?;
        let saved: SavedJson = serde_json::from_str(&document).with_context(|| {
            format!("mem save --json returned an unexpected document: {document}")
        })?;
        if saved.written && saved.already_received {
            bail!("mem save --json says the memory was both written and already there");
        }
        if !saved.written && !saved.already_received {
            bail!("mem save --json did not say whether it wrote the memory");
        }
        Ok(Saved {
            written: saved.written,
            already: saved.already_received,
            id: saved.id,
            same_sender: saved.same_sender,
            from: saved.from,
            delivery_from: saved.delivery_from,
            when: saved.when,
        })
    }

    /// The first line of `mem --version`. `--db` is deliberately not passed:
    /// the question is which binary will run, not which database it reads.
    pub fn version(&self) -> Result<String> {
        let mut command = Command::new(&self.bin);
        command.arg("--version");
        let output = run(command, None, VERSION_TIMEOUT)?;
        if !output.status.success() {
            bail!(
                "mem --version failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.lines().next().unwrap_or("").trim().to_string();
        if line.is_empty() {
            bail!("mem --version printed nothing");
        }
        Ok(line)
    }

    /// How many received memories are still drafts, anywhere in the database.
    /// `--since todo` because a draft is not read within a day by definition,
    /// and the default window is 24 hours.
    pub fn drafts_waiting(&self) -> Result<i64> {
        let mut command = self.command();
        command.args(["recent", "--all", "--external", "--since", "todo", "--json"]);
        let output = run(command, None, RECENT_TIMEOUT)?;
        if !output.status.success() {
            bail!(
                "mem recent --json failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let document = last_json(&output.stdout)?;
        let recent: RecentJson = serde_json::from_str(&document).with_context(|| {
            format!("mem recent --json returned an unexpected document: {document}")
        })?;
        Ok(recent
            .items
            .iter()
            .filter(|item| item.status == "draft")
            .count() as i64)
    }
}

pub struct Saved {
    pub written: bool,
    pub already: bool,
    pub id: i64,
    pub same_sender: Option<bool>,
    pub from: Option<String>,
    pub delivery_from: Option<String>,
    pub when: Option<String>,
}

#[derive(Deserialize)]
struct SavedJson {
    written: bool,
    already_received: bool,
    id: i64,
    #[serde(default)]
    same_sender: Option<bool>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    delivery_from: Option<String>,
    #[serde(default)]
    when: Option<String>,
}

#[derive(Deserialize)]
struct RecentJson {
    items: Vec<RecentItem>,
}

#[derive(Deserialize)]
struct RecentItem {
    status: String,
}

/// Runs a child with a deadline. stdout and stderr are drained by threads so
/// a large `recent --json` cannot deadlock on a full pipe while we wait.
fn run(mut command: Command, stdin_body: Option<&str>, timeout: Duration) -> Result<Output> {
    command
        .stdin(if stdin_body.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("cannot run mem")?;
    if let Some(body) = stdin_body {
        if let Some(mut stdin) = child.stdin.take() {
            // A child that rejects the arguments may exit before reading;
            // the error is in stderr, not in this broken pipe.
            let _ = stdin.write_all(body.as_bytes());
        }
    }
    let mut stdout = child.stdout.take().context("mem did not open stdout")?;
    let mut stderr = child.stderr.take().context("mem did not open stderr")?;
    let out_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).map(|_| buf)
    });
    let err_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        stderr.read_to_end(&mut buf).map(|_| buf)
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_thread.join();
            let _ = err_thread.join();
            bail!("mem did not answer within {} seconds", timeout.as_secs());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = out_thread.join().expect("stdout reader panicked")?;
    let stderr = err_thread.join().expect("stderr reader panicked")?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// One JSON document, possibly after warnings on stdout (none today, but the
/// last non-empty line is the document by contract).
fn last_json(stdout: &[u8]) -> Result<String> {
    let text = String::from_utf8_lossy(stdout);
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .context("mem printed no JSON document")
}

fn old_mem(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    text.contains("--json") && (text.contains("unexpected") || text.contains("unrecognized"))
}

/// Whether the `mem --version` line is at least `minimum`. A version that
/// cannot be read counts as too old: doctor reports that instead of guessing.
pub fn at_least(version_line: &str, minimum: &str) -> bool {
    match (parse_version(version_line), parse_version(minimum)) {
        (Some(found), Some(required)) => found >= required,
        _ => false,
    }
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let token = text
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_non_empty_line_is_the_document() {
        assert_eq!(last_json(b"\n{\"a\":1}\n\n").unwrap(), "{\"a\":1}");
    }

    #[test]
    fn an_old_mem_is_recognised_by_the_json_error() {
        assert!(old_mem("error: unexpected argument '--json' found"));
        assert!(!old_mem("error: there is no database"));
    }

    #[test]
    fn versions_are_compared_as_numbers() {
        assert!(at_least("mem 1.2.0 (commit beaf83f)", "1.2.0"));
        assert!(at_least("mem 1.10.0", "1.2.0"));
        assert!(at_least("mem 2.0", "1.12.9"));
        assert!(!at_least("mem 1.1.9", "1.2.0"));
        assert!(!at_least("not a version", "1.2.0"));
    }
}
