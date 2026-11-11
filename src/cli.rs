//! Argument parsing and the commands that do not talk to the network.

use crate::api::SendRequest;
use crate::client::Api;
use crate::config::{self, ClientConfig};
use crate::pull;
use crate::server;
use crate::store::Store;
use crate::time;
use crate::util::{random_hex, sha256_hex};
use anyhow::{bail, Context, Result};
use lexopt::prelude::*;
use std::io::Read;
use std::path::PathBuf;

pub fn run() -> Result<()> {
    let mut parser = lexopt::Parser::from_env();
    let mut config_path: Option<PathBuf> = None;
    let mut db_path: Option<PathBuf> = None;
    let mut command: Option<String> = None;

    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => {
                print_help();
                return Ok(());
            }
            Short('V') | Long("version") => {
                println!("stele {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Long("config") => config_path = Some(PathBuf::from(parser.value()?.string()?)),
            Long("db") => db_path = Some(PathBuf::from(parser.value()?.string()?)),
            Value(value) => {
                command = Some(value.string()?);
                break;
            }
            _ => return Err(arg.unexpected().into()),
        }
    }

    let Some(command) = command else {
        print_help();
        bail!("no command given");
    };
    match command.as_str() {
        "serve" => serve(&mut parser, db_path),
        "token" => token(&mut parser, db_path),
        "login" => login(&mut parser, config_path),
        "whoami" => whoami(&mut parser, config_path),
        "peers" => peers(&mut parser, config_path),
        "status" => status(&mut parser, config_path),
        "send" => send(&mut parser, config_path),
        "pull" => pull_command(&mut parser, config_path),
        other => bail!("unknown command {other:?}; run `stele --help`"),
    }
}

fn print_help() {
    println!(
        "\
stele {} — a mailbox between coding agents

usage:
  stele [--config PATH] login --url URL --token TOKEN|-
  stele [--config PATH] whoami
  stele [--config PATH] peers
  stele [--config PATH] status
  stele [--config PATH] send --to MAILBOX --project PROJECT [BODY|-]
  stele [--config PATH] pull [--dry-run] [--limit N] [--mem PATH] [--mem-db PATH]
  stele [--db PATH] serve [--bind ADDR] [--verbose]
  stele [--db PATH] token add|rotate|list|remove [MAILBOX]

Commands:
  login    validates the token against the server and writes the config (0600)
  whoami   prints the mailbox this machine is
  peers    lists the mailboxes the server knows
  status   server, mailbox and how many messages are waiting
  send     queues one message; the body may come from stdin (`-`)
  pull     fetches unread messages and writes each as an anathem draft
  serve    runs the mailbox; it listens on plain HTTP and is meant for a tailnet
  token    creates, rotates, lists or removes the server's mailboxes

Environment:
  STELE_CONFIG   where the client config lives (default ~/.config/stele/config.json)
  STELE_DB       the server database (default ~/.local/share/stele/stele.db)",
        env!("CARGO_PKG_VERSION")
    );
}

fn serve(parser: &mut lexopt::Parser, db: Option<PathBuf>) -> Result<()> {
    let mut bind = "127.0.0.1:3900".to_string();
    let mut verbose = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("bind") => bind = parser.value()?.string()?,
            Long("verbose") => verbose = true,
            _ => return Err(arg.unexpected().into()),
        }
    }
    let db = db.unwrap_or(default_db_path()?);
    server::serve(&bind, &db, verbose)
}

fn token(parser: &mut lexopt::Parser, db: Option<PathBuf>) -> Result<()> {
    let mut values: Vec<String> = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Value(value) => values.push(value.string()?),
            _ => return Err(arg.unexpected().into()),
        }
    }
    let action = values
        .first()
        .map(String::as_str)
        .context("`stele token` needs a subcommand: add, rotate, list or remove")?;
    let db = db.unwrap_or(default_db_path()?);
    let store = Store::open(&db)?;
    match action {
        "list" => {
            if values.len() > 1 {
                bail!("`stele token list` takes no arguments");
            }
            for name in store.mailboxes()? {
                println!("{name}");
            }
        }
        "add" | "rotate" => {
            let name = values
                .get(1)
                .context("which mailbox? `stele token add <name>`")?;
            check_mailbox(name)?;
            let token = random_hex(32)?;
            let hash = sha256_hex(&token);
            if action == "add" {
                store.add_mailbox(name, &hash, time::now())?;
            } else {
                store.rotate_mailbox(name, &hash)?;
            }
            // The token goes to stdout alone, so it can be piped into a config
            // file; the instructions go to stderr where they do not pollute it.
            println!("{token}");
            eprintln!(
                "mailbox {name:?} {}; on that machine: stele login --url URL --token -",
                if action == "add" {
                    "created"
                } else {
                    "rotated"
                }
            );
        }
        "remove" => {
            let name = values
                .get(1)
                .context("which mailbox? `stele token remove <name>`")?;
            if store.remove_mailbox(name)? {
                println!("removed {name:?}");
            } else {
                bail!("there is no mailbox {name:?}");
            }
        }
        other => bail!("unknown token action {other:?}"),
    }
    Ok(())
}

fn login(parser: &mut lexopt::Parser, config_path: Option<PathBuf>) -> Result<()> {
    let mut url = None;
    let mut token = None;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("url") => url = Some(parser.value()?.string()?),
            Long("token") => {
                let value = parser.value()?.string()?;
                token = Some(if value == "-" { read_stdin()? } else { value });
            }
            _ => return Err(arg.unexpected().into()),
        }
    }
    let url = url.context("`stele login` needs --url http://host:port")?;
    let token =
        token.context("`stele login` needs --token (use `--token -` to read it from stdin)")?;
    let token = token.trim().to_string();
    if token.is_empty() {
        bail!("the token is empty");
    }
    let api = Api::new(&url, token.clone())?;
    let me = api.me()?;
    let path = config_path.unwrap_or(config::default_path()?);
    config::save(&path, &ClientConfig::new(url, me.mailbox.clone(), token))?;
    println!(
        "logged in as {:?} at {}",
        me.mailbox, api.endpoint.authority
    );
    println!("config: {}", path.display());
    Ok(())
}

fn whoami(parser: &mut lexopt::Parser, config_path: Option<PathBuf>) -> Result<()> {
    no_arguments(parser)?;
    let config = load_config(config_path)?;
    let api = Api::new(&config.url, config.token)?;
    println!("{}", api.me()?.mailbox);
    Ok(())
}

fn peers(parser: &mut lexopt::Parser, config_path: Option<PathBuf>) -> Result<()> {
    no_arguments(parser)?;
    let config = load_config(config_path)?;
    let api = Api::new(&config.url, config.token)?;
    for mailbox in api.peers()?.mailboxes {
        println!("{mailbox}");
    }
    Ok(())
}

fn status(parser: &mut lexopt::Parser, config_path: Option<PathBuf>) -> Result<()> {
    no_arguments(parser)?;
    let config = load_config(config_path)?;
    let api = Api::new(&config.url, config.token)?;
    let status = api.status()?;
    println!("server:  {}", api.endpoint.authority);
    println!("mailbox: {}", status.mailbox);
    println!("pending: {}", status.pending);
    Ok(())
}

fn send(parser: &mut lexopt::Parser, config_path: Option<PathBuf>) -> Result<()> {
    let mut to = None;
    let mut project = None;
    let mut client_id = None;
    let mut body = None;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("to") => to = Some(parser.value()?.string()?),
            Long("project") => project = Some(parser.value()?.string()?),
            Long("client-id") => client_id = Some(parser.value()?.string()?),
            Value(value) => {
                if body.is_some() {
                    bail!("only one body, and it has to come last");
                }
                body = Some(value.string()?);
            }
            _ => return Err(arg.unexpected().into()),
        }
    }
    let to = to.context("`stele send` needs --to MAILBOX")?;
    let project = project.context("`stele send` needs --project PROJECT")?;
    let body = match body.as_deref() {
        Some("-") => read_stdin()?,
        Some(text) => text.to_string(),
        None => {
            // Reading stdin is what makes `printf ... | stele send ...` work,
            // but in a terminal it would just wait for an EOF nobody types.
            use std::io::IsTerminal;
            if std::io::stdin().is_terminal() {
                bail!("give the body as the last argument, or pipe it with `-`");
            }
            read_stdin()?
        }
    };
    if body.trim().is_empty() {
        bail!("the message body is empty");
    }

    let config = load_config(config_path)?;
    let api = Api::new(&config.url, config.token)?;
    let request = SendRequest {
        client_id: client_id.unwrap_or(random_hex(12)?),
        to: to.trim().to_string(),
        project: project.trim().to_string(),
        body,
    };
    let response = api.send(&request)?;
    if response.duplicate {
        println!("already queued as {}", response.id);
    } else {
        println!("queued as {}", response.id);
    }
    Ok(())
}

fn pull_command(parser: &mut lexopt::Parser, config_path: Option<PathBuf>) -> Result<()> {
    let mut dry_run = false;
    let mut limit = 50;
    let mut mem_bin = None;
    let mut mem_db = None;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("dry-run") => dry_run = true,
            Long("limit") => {
                limit = parser.value()?.parse().context("--limit takes a number")?;
            }
            Long("mem") => mem_bin = Some(parser.value()?.string()?),
            Long("mem-db") => mem_db = Some(parser.value()?.string()?),
            _ => return Err(arg.unexpected().into()),
        }
    }
    if !(1..=crate::api::MAX_PENDING).contains(&limit) {
        bail!(
            "--limit has to be between 1 and {}",
            crate::api::MAX_PENDING
        );
    }
    let config = load_config(config_path)?;
    let api = Api::new(&config.url, config.token)?;
    let options = pull::Options {
        dry_run,
        limit,
        mem_bin: mem_bin.or(config.mem_bin),
        mem_db: mem_db.or(config.mem_db),
    };
    let count = pull::run(&api, &options)?;
    if count == 0 && !dry_run {
        // Quiet on purpose: a timer that runs every five minutes should not
        // write a line saying that nothing happened.
    }
    Ok(())
}

fn no_arguments(parser: &mut lexopt::Parser) -> Result<()> {
    if let Some(arg) = parser.next()? {
        return Err(arg.unexpected().into());
    }
    Ok(())
}

fn load_config(path: Option<PathBuf>) -> Result<ClientConfig> {
    let path = match path {
        Some(p) => p,
        None => config::default_path()?,
    };
    config::load(&path)
}

fn read_stdin() -> Result<String> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("cannot read stdin")?;
    Ok(text)
}

fn check_mailbox(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("the mailbox name is empty");
    }
    if name.len() > crate::api::MAX_MAILBOX {
        bail!("the mailbox name is over {} bytes", crate::api::MAX_MAILBOX);
    }
    if name.chars().any(|c| c.is_control()) {
        bail!("the mailbox name has control characters");
    }
    Ok(())
}

fn default_db_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("STELE_DB") {
        return Ok(PathBuf::from(p));
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("stele").join("stele.db"));
        }
    }
    let home = std::env::var_os("HOME").context("HOME is not set; pass --db")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("stele")
        .join("stele.db"))
}
