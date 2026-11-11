//! End to end over a real socket: two mailboxes, one server, the actual
//! binary, and a fake `mem` that records what `pull` hands it.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use stele::http::{call, Endpoint};

struct Server {
    child: Child,
    url: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn stele(args: &[&str], env: &[(&str, &Path)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stele"));
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}

fn ok(output: Output) -> String {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn token(db: &Path, mailbox: &str) -> String {
    ok(stele(
        &["--db", db.to_str().unwrap(), "token", "add", mailbox],
        &[],
    ))
}

fn start(db: &Path) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stele"))
        .args([
            "--db",
            db.to_str().unwrap(),
            "serve",
            "--bind",
            "127.0.0.1:0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let url = line
        .trim()
        .strip_prefix("listening on ")
        .expect("the server did not announce its address")
        .to_string();
    Server { child, url }
}

struct Box {
    config: PathBuf,
}

fn login(server: &Server, dir: &Path, mailbox: &str, token: &str) -> Box {
    let config = dir.join(format!("{mailbox}.json"));
    ok(stele(
        &[
            "--config",
            config.to_str().unwrap(),
            "login",
            "--url",
            &server.url,
            "--token",
            token,
        ],
        &[],
    ));
    Box { config }
}

fn send(from: &Box, to: &str, project: &str, body: &str) -> Output {
    stele(
        &[
            "--config",
            from.config.to_str().unwrap(),
            "send",
            "--to",
            to,
            "--project",
            project,
            body,
        ],
        &[],
    )
}

fn pull(me: &Box, mem: &Path, log: &Path, extra: &[&str]) -> Output {
    let mut args = vec![
        "--config",
        me.config.to_str().unwrap(),
        "pull",
        "--mem",
        mem.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    stele(&args, &[("MEM_LOG", log)])
}

fn fake_mem(dir: &Path) -> PathBuf {
    let path = dir.join("mem");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "#!/bin/sh").unwrap();
    writeln!(
        file,
        "{{ printf 'ARGS %s\\n' \"$*\"; cat; printf '\\nEND\\n'; }} >> \"$MEM_LOG\""
    )
    .unwrap();
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[test]
fn a_message_travels_as_a_draft_and_is_acknowledged_once() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let laptop_token = token(&db, "laptop");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let laptop = login(&server, dir.path(), "laptop", &laptop_token);

    let sent = send(&laptop, "dell", "api", "The API contract changed to v2");
    let sent = ok(sent);
    assert!(sent.starts_with("queued as "), "{sent}");
    let message_id = sent.trim_start_matches("queued as ").to_string();

    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");
    let dry = pull(&dell, &mem, &log, &["--dry-run"]);
    let dry = ok(dry);
    assert!(dry.contains("laptop -> api"), "{dry}");
    assert!(dry.contains("The API contract changed to v2"), "{dry}");
    assert!(!log.exists(), "a dry run must not call mem");

    let first = ok(pull(&dell, &mem, &log, &[]));
    assert!(first.contains("draft "), "{first}");
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert!(
        recorded.contains("save - --external --from laptop --project api"),
        "{recorded}"
    );
    assert!(
        recorded.contains(&format!("--ext-id stele:{message_id}")),
        "the retry key has to be the server's message id: {recorded}"
    );
    assert!(
        recorded.contains("The API contract changed to v2"),
        "{recorded}"
    );

    let second = ok(pull(&dell, &mem, &log, &[]));
    assert!(
        second.is_empty(),
        "a message must not be delivered twice: {second}"
    );
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .matches("ARGS")
            .count(),
        1
    );
}

#[test]
fn retrying_with_the_same_client_id_does_not_duplicate() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let tokens = (token(&db, "dell"), token(&db, "laptop"));
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &tokens.0);
    let laptop = login(&server, dir.path(), "laptop", &tokens.1);

    let send_once = || {
        stele(
            &[
                "--config",
                laptop.config.to_str().unwrap(),
                "send",
                "--to",
                "dell",
                "--project",
                "api",
                "--client-id",
                "retry-me",
                "one body",
            ],
            &[],
        )
    };
    let first = ok(send_once());
    let second = ok(send_once());
    assert!(first.starts_with("queued as "), "{first}");
    assert!(second.starts_with("already queued as "), "{second}");

    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");
    ok(pull(&dell, &mem, &log, &[]));
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .matches("ARGS")
            .count(),
        1
    );
}

#[test]
fn the_token_is_checked_and_the_config_is_private() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dell.config)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let bad = dir.path().join("bad.json");
    let failed = stele(
        &[
            "--config",
            bad.to_str().unwrap(),
            "login",
            "--url",
            &server.url,
            "--token",
            "not-a-token",
        ],
        &[],
    );
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("401"));
}

#[test]
fn unknown_recipients_and_empty_bodies_are_refused() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);

    let missing = send(&dell, "nobody", "api", "hello");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("404"));

    let empty = stele(
        &[
            "--config",
            dell.config.to_str().unwrap(),
            "send",
            "--to",
            "dell",
            "--project",
            "api",
        ],
        &[],
    );
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("empty"));
}

#[test]
fn peers_lists_the_mailboxes() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let laptop_token = token(&db, "laptop");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let _laptop = login(&server, dir.path(), "laptop", &laptop_token);

    let listed = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "peers"],
        &[],
    ));
    assert_eq!(listed, "dell\nlaptop");
}

#[test]
fn an_oversized_body_is_refused_before_it_is_stored() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);

    let body = "x".repeat(stele::api::MAX_BODY + 1);
    let refused = send(&dell, "dell", "api", &body);
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("413"), "{stderr}");

    // And nothing made it into the mailbox.
    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");
    let pulled = ok(pull(&dell, &mem, &log, &[]));
    assert!(pulled.is_empty(), "{pulled}");
}

#[test]
fn the_sender_comes_from_the_token_not_the_body() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let endpoint = Endpoint::parse(&server.url).unwrap();

    // Both spellings someone might try; serde ignores what the type does not
    // declare, and the server has no field to read them from anyway.
    let payload = serde_json::json!({
        "client_id": "forged",
        "to": "dell",
        "project": "api",
        "body": "this claims another sender",
        "from": "mallory",
        "from_agent": "mallory"
    });
    let (status, raw) = call(
        &endpoint,
        "POST",
        "/messages",
        Some(&dell_token),
        Some(&serde_json::to_vec(&payload).unwrap()),
    )
    .unwrap();
    assert_eq!(status, 201, "{}", String::from_utf8_lossy(&raw));

    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");
    let dry = ok(pull(&dell, &mem, &log, &["--dry-run"]));
    assert!(
        dry.contains("dell -> api"),
        "filed under the token's mailbox: {dry}"
    );
    assert!(
        !dry.contains("mallory"),
        "a body cannot sign as another mailbox: {dry}"
    );
}

#[test]
fn another_mailbox_cannot_read_this_inbox() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let laptop_token = token(&db, "laptop");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let _laptop = login(&server, dir.path(), "laptop", &laptop_token);
    ok(send(&dell, "dell", "api", "for dell only"));

    let endpoint = Endpoint::parse(&server.url).unwrap();
    let (status, raw) = call(
        &endpoint,
        "GET",
        "/messages?to=dell&unread=1",
        Some(&laptop_token),
        None,
    )
    .unwrap();
    assert_eq!(status, 200);
    let inbox: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(inbox["messages"].as_array().unwrap().len(), 0);

    // The parameter is ignored in both directions: the token decides.
    let (status, raw) = call(
        &endpoint,
        "GET",
        "/messages?to=laptop&unread=1",
        Some(&dell_token),
        None,
    )
    .unwrap();
    assert_eq!(status, 200);
    let inbox: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(inbox["messages"].as_array().unwrap().len(), 1);
}

#[test]
fn two_mailboxes_cannot_share_a_name() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let _ = token(&db, "dell");
    let again = stele(&["--db", db.to_str().unwrap(), "token", "add", "dell"], &[]);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("already exists"));
}
