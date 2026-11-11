//! `stele pull`: fetch what is addressed to this mailbox and hand each body to
//! `mem save --external`, so what arrives is a draft and never active memory.
//!
//! A message is acknowledged only after `mem` exits zero. With anathem 0.10 or
//! newer the body also carries `--ext-id stele:<message id>`, so a retry after
//! a lost acknowledgement collapses into the draft that already exists instead
//! of writing a second one. Without that flag the failure mode is a duplicate
//! someone can delete, which still beats a note nobody knows about.

use crate::api::Message;
use crate::client::Api;
use crate::time;
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct Options {
    pub dry_run: bool,
    pub limit: i64,
    pub mem_bin: Option<String>,
    pub mem_db: Option<String>,
}

pub fn run(api: &Api, options: &Options) -> Result<usize> {
    let messages = api.pull(options.limit)?;
    if messages.is_empty() {
        return Ok(0);
    }
    let mut delivered = 0;
    let mut failed = 0;
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
        match save_draft(api, message, options) {
            Ok(()) => {
                delivered += 1;
                println!(
                    "draft {} from {} ({})",
                    message.id, message.from, message.project
                );
            }
            // One note anathem refuses must not hold the queue behind it.
            // The failed message stays unread and is retried next time; the
            // rest are delivered and acknowledged now.
            Err(error) => {
                failed += 1;
                if shown < 5 {
                    eprintln!("stele: {error:#}");
                    shown += 1;
                }
            }
        }
    }
    if failed > 0 {
        bail!(
            "{failed} of {} messages could not be delivered; they stay unread",
            messages.len()
        );
    }
    Ok(if options.dry_run {
        messages.len()
    } else {
        delivered
    })
}

fn save_draft(api: &Api, message: &Message, options: &Options) -> Result<()> {
    let mem = options.mem_bin.as_deref().unwrap_or("mem");
    let mut command = Command::new(mem);
    if let Some(db) = &options.mem_db {
        command.arg("--db").arg(db);
    }
    command.args([
        "save",
        "-",
        "--external",
        "--from",
        &message.from,
        "--project",
        &message.project,
    ]);
    command.arg("--ext-id").arg(ext_id(message));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().with_context(|| {
        format!("cannot run {mem:?}; pass --mem or set \"mem_bin\" in the config")
    })?;
    {
        let mut stdin = child.stdin.take().context("mem did not open stdin")?;
        stdin
            .write_all(message.body.as_bytes())
            .context("cannot send the message body to mem")?;
    }
    // Captured and reprinted instead of inherited: on failure the child's own
    // error is still what the person sees, and stele can add the one sentence
    // that turns "unexpected argument" into an upgrade instruction.
    let output = child.wait_with_output().context("waiting for mem")?;
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;
    if !output.status.success() {
        let hint = if old_mem(&output.stderr) {
            " (stele needs anathem 0.10 or newer: this `mem` does not know --ext-id)"
        } else {
            ""
        };
        bail!(
            "mem save failed for message {} ({}){hint}; it stays unread",
            message.id,
            output.status
        );
    }
    api.ack(&message.id).with_context(|| {
        format!(
            "the draft was saved but message {} could not be acknowledged; it will arrive again",
            message.id
        )
    })?;
    Ok(())
}

/// The key anathem stores in `memories.ext_id` to tell a retry from a new
/// message. Namespaced because that unique column is shared with the document
/// ids `mem migrate` writes; the server's message id is already random and
/// stable.
fn ext_id(message: &Message) -> String {
    format!("stele:{}", message.id)
}

fn old_mem(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    text.contains("--ext-id") && (text.contains("unexpected") || text.contains("unrecognized"))
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

    #[test]
    fn an_old_mem_is_recognised_by_its_error() {
        assert!(old_mem(
            b"error: unexpected argument '--ext-id' found\n\nUsage: mem save ..."
        ));
        assert!(!old_mem(b"error: there is no database"));
    }
}
