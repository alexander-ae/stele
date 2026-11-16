//! `stele pull`: fetch what is addressed to this mailbox and hand each body to
//! `mem save --external --json`, so what arrives is a draft and never active
//! memory.
//!
//! A message is acknowledged only after `mem` exits zero. With anathem 1.2 or
//! newer the body also carries `--ext-id stele:<message id>`, so a retry after
//! a lost acknowledgement collapses into the draft that already exists, and
//! `--json` says whether this delivery wrote a draft or found one.

use crate::api::Message;
use crate::client::Api;
use crate::failure::{self, Kind};
use crate::mem::Mem;
use crate::time;
use anyhow::{bail, Context, Result};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// A notifier that hangs must not hold the timer that runs `pull`.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Options {
    pub dry_run: bool,
    pub limit: i64,
    pub mem_bin: Option<String>,
    pub mem_db: Option<String>,
}

#[derive(Default)]
pub struct Summary {
    /// Drafts this pull wrote.
    pub written: usize,
    /// Deliveries whose `--ext-id` was already here: a retry.
    pub already: usize,
    /// Messages anathem refused; they stay unread.
    pub failed: usize,
    pub total: usize,
}

pub fn run(api: &Api, options: &Options) -> Result<Summary> {
    let messages = api.pull(options.limit)?;
    if messages.is_empty() {
        return Ok(Summary::default());
    }
    let mem = Mem::new(options.mem_bin.clone(), options.mem_db.clone());
    let mut summary = Summary {
        total: messages.len(),
        ..Summary::default()
    };
    let mut shown = 0;

    for message in &messages {
        if options.dry_run {
            println!(
                "{}  {} -> {}  {}  {}",
                time::iso8601(message.created),
                message.from,
                message.project,
                message.id,
                first_line(&message.body)
            );
            continue;
        }
        match save_draft(api, message, &mem) {
            Ok(saved) => {
                if saved.written {
                    summary.written += 1;
                    println!(
                        "draft {} from {} ({})",
                        message.id, message.from, message.project
                    );
                } else {
                    summary.already += 1;
                    println!(
                        "already here {} (memory {}) from {} ({})",
                        message.id, saved.id, message.from, message.project
                    );
                }
                if saved.same_sender == Some(false) {
                    let who = saved.from.as_deref().unwrap_or("another sender");
                    let delivery = saved.delivery_from.as_deref().unwrap_or("this delivery");
                    eprintln!(
                        "stele: message {} reuses the --ext-id of memory {} from {who} \
                         (delivery from {delivery}); one of the two was dropped",
                        message.id, saved.id
                    );
                }
            }
            // One note anathem refuses must not hold the queue behind it.
            // The failed message stays unread and is retried next time; the
            // rest are delivered and acknowledged now.
            Err(error) => {
                summary.failed += 1;
                if shown < 5 {
                    eprintln!("stele: {error:#}");
                    shown += 1;
                }
                // If the server is gone or asking for time, the next message
                // will fail the same way; stop instead of waiting out the
                // timeout once per message.
                if matches!(
                    failure::kind(&error),
                    Some(Kind::Unreachable | Kind::Temporary)
                ) {
                    return Err(error);
                }
            }
        }
    }
    Ok(summary)
}

fn save_draft(api: &Api, message: &Message, mem: &Mem) -> Result<crate::mem::Saved> {
    let saved = mem
        .save_external(
            &message.body,
            &message.from,
            &message.project,
            &ext_id(message),
        )
        .with_context(|| format!("message {} stays unread", message.id))?;
    api.ack(&message.id).with_context(|| {
        format!(
            "the draft was saved but message {} could not be acknowledged; it will arrive again",
            message.id
        )
    })?;
    Ok(saved)
}

/// The key anathem stores in `memories.ext_id` to tell a retry from a new
/// message. Namespaced because that unique column is shared with the document
/// ids `mem migrate` writes; the server's message id is already random and
/// stable.
fn ext_id(message: &Message) -> String {
    format!("stele:{}", message.id)
}

/// Runs the optional notifier. The command is the operator's, and the only
/// things it receives are counts and the mailbox: a received note cannot
/// write itself into a command line.
pub fn notify(
    command: &str,
    mailbox: &str,
    written: usize,
    already: usize,
    pending: Option<i64>,
) -> Result<()> {
    let mut child = Command::new("sh");
    child
        .arg("-c")
        .arg(command)
        .env("STELE_DRAFTS", written.to_string())
        .env("STELE_ALREADY", already.to_string())
        .env("STELE_MAILBOX", mailbox)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(pending) = pending {
        child.env("STELE_PENDING", pending.to_string());
    }
    let mut child = child
        .spawn()
        .with_context(|| format!("cannot run notify_cmd {command:?}"))?;
    let deadline = Instant::now() + NOTIFY_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("notify_cmd exited with {status}");
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "notify_cmd did not finish within {} seconds",
                NOTIFY_TIMEOUT.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn first_line(body: &str) -> String {
    body.lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(100)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ext_id_is_namespaced_and_stable() {
        let message = Message {
            id: "0123456789abcdef".into(),
            from: "laptop".into(),
            project: "api".into(),
            body: "hi".into(),
            created: 0,
        };
        assert_eq!(ext_id(&message), "stele:0123456789abcdef");
        assert_eq!(ext_id(&message), ext_id(&message));
    }
}
