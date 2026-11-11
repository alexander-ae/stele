//! `stele pull`: fetch what is addressed to this mailbox and hand each body to
//! `mem save --external`, so what arrives is a draft and never active memory.
//!
//! Delivery is at-least-once. A message is acknowledged only after `mem` exits
//! zero; if the process dies in between, the message comes back and anathem
//! gets a second draft. That is the honest failure mode: a duplicate someone
//! can delete beats a note nobody knows about.

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
        save_draft(api, message, options)?;
        println!(
            "draft {} from {} ({})",
            message.id, message.from, message.project
        );
    }
    Ok(messages.len())
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
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command.spawn().with_context(|| {
        format!("cannot run {mem:?}; pass --mem or set \"mem_bin\" in the config")
    })?;
    {
        let mut stdin = child.stdin.take().context("mem did not open stdin")?;
        stdin
            .write_all(message.body.as_bytes())
            .context("cannot send the message body to mem")?;
    }
    let status = child.wait().context("waiting for mem")?;
    if !status.success() {
        bail!(
            "mem save failed for message {} ({status}); it stays unread",
            message.id
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

fn first_line(body: &str) -> String {
    body.lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(100)
        .collect()
}
