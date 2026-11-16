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

const FAKE_MEM: &str = r#"#!/bin/sh
if [ "${1:-}" = "--db" ]; then shift 2; fi
cmd="${1:-}"
marker='__FAIL_ON__'
case "$cmd" in
  --version)
    printf 'mem 1.2.0 (fake)\n'
    ;;
  save)
    body=$(cat)
    if [ -n "$marker" ] && printf '%s' "$body" | grep -q "$marker"; then
      echo FAIL >> "$MEM_LOG"
      echo 'anathem refused this note' >&2
      exit 1
    fi
    { printf 'ARGS %s\n' "$*"; printf '%s\n' "$body"; printf 'END\n'; } >> "$MEM_LOG"
    ext=""
    prev=""
    for a in "$@"; do
      if [ "$prev" = "--ext-id" ]; then ext="$a"; fi
      prev="$a"
    done
    ids="$MEM_LOG.ids"
    touch "$ids"
    if grep -qxF "$ext" "$ids" 2>/dev/null; then
      printf '{"already_received":true,"delivery_from":"laptop","from":"laptop","id":1,"same_sender":true,"schema":1,"when":"2026-01-01T00:00:00Z","written":false}\n'
    else
      printf '%s\n' "$ext" >> "$ids"
      n=$(wc -l < "$ids")
      printf '{"already_received":false,"from":"laptop","id":%s,"project":"api","schema":1,"status":"draft","written":true}\n' "$n"
    fi
    ;;
  recent)
    n=$(cat "$MEM_LOG.drafts" 2>/dev/null || echo 0)
    printf '{"items":['
    i=1
    while [ "$i" -le "$n" ]; do
      [ "$i" -gt 1 ] && printf ','
      printf '{"id":%s,"status":"draft"}' "$i"
      i=$((i+1))
    done
    printf '],"schema":1}\n'
    ;;
  *)
    printf '{}\n'
    ;;
esac
"#;

fn write_mem(dir: &Path, name: &str, marker: &str) -> PathBuf {
    let path = dir.join(name);
    let script = FAKE_MEM.replace("__FAIL_ON__", marker);
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn fake_mem(dir: &Path) -> PathBuf {
    write_mem(dir, "mem", "")
}

fn fake_mem_failing_on(dir: &Path, marker: &str) -> PathBuf {
    write_mem(dir, "mem-fail", marker)
}

fn set_mem_bin(config: &Path, bin: &Path) {
    let text = std::fs::read_to_string(config).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["mem_bin"] = serde_json::json!(bin.to_str().unwrap());
    std::fs::write(config, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

fn set_token(config: &Path, token: &str) {
    let text = std::fs::read_to_string(config).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["token"] = serde_json::json!(token);
    std::fs::write(config, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

/// A `mem` that answers `--version` and refuses everything else, for the
/// checks doctor makes before it trusts the rest.
fn fake_mem_version(dir: &Path, name: &str, version: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!("#!/bin/sh\nif [ \"${{1:-}}\" = \"--version\" ]; then echo 'mem {version}'; exit 0; fi\nexit 1\n"),
    )
    .unwrap();
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
    assert_eq!(refused.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("the limit is 32768"), "{stderr}");

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

#[test]
fn a_body_at_the_cap_is_accepted_and_one_byte_over_is_not() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);

    let at_cap = "x".repeat(stele::api::MAX_BODY);
    let sent = ok(send(&dell, "dell", "api", &at_cap));
    assert!(sent.starts_with("queued as "), "{sent}");

    let over = send(&dell, "dell", "api", &format!("{at_cap}x"));
    assert_eq!(over.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&over.stderr).contains("the limit is 32768"),
        "one byte over the body cap has to be refused"
    );
}

#[test]
fn a_full_inbox_of_large_notes_drains_in_batches() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let endpoint = Endpoint::parse(&server.url).unwrap();

    // 70 * 32.6 KB is 2.28 MB: more than the client's 2 MB response cap, and
    // far more than one 1 MB batch. Before the byte budget this made the first
    // pull fail with "response is too large" and the inbox never drained.
    let body = "x".repeat(32_600);
    for i in 0..70 {
        let payload = serde_json::json!({
            "client_id": format!("batch-{i}"),
            "to": "dell",
            "project": "api",
            "body": body
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
    }

    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");
    let mut pulls = 0;
    loop {
        let out = ok(pull(&dell, &mem, &log, &["--limit", "200"]));
        pulls += 1;
        if out.is_empty() {
            break;
        }
        assert!(pulls < 10, "the inbox did not drain in batches");
    }
    assert!(pulls >= 2, "70 large notes cannot fit in one batch");
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .matches("ARGS")
            .count(),
        70
    );
    assert!(ok(pull(&dell, &mem, &log, &[])).is_empty());
}

#[test]
fn one_bad_message_does_not_block_the_rest() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    ok(send(
        &dell,
        "dell",
        "api",
        "POISON: anathem will refuse this",
    ));
    ok(send(&dell, "dell", "api", "good one"));
    ok(send(&dell, "dell", "api", "good two"));

    let mem = fake_mem_failing_on(dir.path(), "POISON");
    let log = dir.path().join("mem.log");

    let first = pull(&dell, &mem, &log, &[]);
    assert!(!first.status.success(), "one failure has to be reported");
    let stderr = String::from_utf8_lossy(&first.stderr);
    assert!(stderr.contains("1 of 3"), "{stderr}");

    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(recorded.matches("ARGS").count(), 2, "{recorded}");
    assert_eq!(recorded.matches("FAIL").count(), 1, "{recorded}");

    // The good ones are acknowledged; the second pull retries only the bad one.
    let second = pull(&dell, &mem, &log, &[]);
    assert!(!second.status.success());
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        recorded.matches("ARGS").count(),
        2,
        "acknowledged messages must not come back: {recorded}"
    );
    assert_eq!(
        recorded.matches("FAIL").count(),
        2,
        "the failed message is retried: {recorded}"
    );
}

#[test]
fn status_shows_the_mailbox_and_pending() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let laptop_token = token(&db, "laptop");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let laptop = login(&server, dir.path(), "laptop", &laptop_token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&dell.config, &mem);
    set_mem_bin(&laptop.config, &mem);

    ok(send(&laptop, "dell", "api", "first"));
    ok(send(&laptop, "dell", "api", "second"));
    ok(send(&dell, "laptop", "api", "for the laptop"));

    let dell_status = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "status"],
        &[],
    ));
    assert!(dell_status.contains("mailbox: dell"), "{dell_status}");
    assert!(dell_status.contains("unread:  2"), "{dell_status}");

    let laptop_status = ok(stele(
        &["--config", laptop.config.to_str().unwrap(), "status"],
        &[],
    ));
    assert!(laptop_status.contains("mailbox: laptop"), "{laptop_status}");
    assert!(laptop_status.contains("unread:  1"), "{laptop_status}");
}

#[test]
fn identifiers_follow_the_rule_of_the_field() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let _dell = login(&server, dir.path(), "dell", &dell_token);
    let endpoint = Endpoint::parse(&server.url).unwrap();

    let post = |body: serde_json::Value| {
        call(
            &endpoint,
            "POST",
            "/messages",
            Some(&dell_token),
            Some(&serde_json::to_vec(&body).unwrap()),
        )
        .unwrap()
    };

    // Mailbox and client_id are identifiers with a narrow charset.
    for (field, value) in [("to", "dell x"), ("client_id", "a b")] {
        let mut body = serde_json::json!({
            "client_id": "ok",
            "to": "dell",
            "project": "api",
            "body": "x"
        });
        body[field] = serde_json::json!(value);
        let (status, raw) = post(body);
        assert_eq!(
            status,
            400,
            "{field}={value:?} should be refused: {}",
            String::from_utf8_lossy(&raw)
        );
    }

    // Project names follow anathem: free text without control characters.
    for (i, project) in ["api & web", "cliente: acme", "ñandú", "org/repo"]
        .iter()
        .enumerate()
    {
        let body = serde_json::json!({
            "client_id": format!("ok-project-{i}"),
            "to": "dell",
            "project": project,
            "body": "x"
        });
        let (status, raw) = post(body);
        assert_eq!(
            status,
            201,
            "project {project:?} should be accepted: {}",
            String::from_utf8_lossy(&raw)
        );
    }

    let control = post(serde_json::json!({
        "client_id": "bad-project",
        "to": "dell",
        "project": "api\nx",
        "body": "x"
    }));
    assert_eq!(control.0, 400, "a control character is still refused");

    // Dots and @ are part of real mailbox names.
    let valid = post(serde_json::json!({
        "client_id": "ok-1",
        "to": "dell",
        "project": "kipin.pe",
        "body": "x"
    }));
    assert_eq!(valid.0, 201, "{}", String::from_utf8_lossy(&valid.1));
}

#[test]
fn a_mailbox_name_can_carry_a_dot_and_an_at() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let token = token(&db, "claude-code@laptop");
    let server = start(&db);
    let config = login(&server, dir.path(), "claude-code@laptop", &token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&config.config, &mem);
    ok(send(&config, "claude-code@laptop", "kipin.pe", "hello"));
    let status = ok(stele(
        &["--config", config.config.to_str().unwrap(), "status"],
        &[],
    ));
    assert!(status.contains("mailbox: claude-code@laptop"), "{status}");
    assert!(status.contains("unread:  1"), "{status}");
}

#[test]
fn token_add_refuses_a_mailbox_name_with_a_space() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let refused = stele(
        &["--db", db.to_str().unwrap(), "token", "add", "bad name"],
        &[],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("letters"));
}

#[test]
fn discard_requires_yes_and_removes_only_that_message() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&dell.config, &mem);

    let keep = ok(send(&dell, "dell", "api", "keep me"));
    let keep_id = keep.trim_start_matches("queued as ");
    assert!(!keep_id.is_empty());
    let drop = ok(send(&dell, "dell", "api", "discard me"));
    let drop_id = drop.trim_start_matches("queued as ");

    let refused = stele(
        &[
            "--config",
            dell.config.to_str().unwrap(),
            "discard",
            drop_id,
        ],
        &[],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("pass --yes"));

    let status = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "status"],
        &[],
    ));
    assert!(status.contains("unread:  2"), "{status}");

    let done = ok(stele(
        &[
            "--config",
            dell.config.to_str().unwrap(),
            "discard",
            "--yes",
            drop_id,
        ],
        &[],
    ));
    assert!(done.contains("discarded"), "{done}");
    let status = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "status"],
        &[],
    ));
    assert!(status.contains("unread:  1"), "{status}");

    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");
    ok(pull(&dell, &mem, &log, &[]));
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("keep me"), "{recorded}");
    assert!(!recorded.contains("discard me"), "{recorded}");
    assert_eq!(recorded.matches("ARGS").count(), 1);
}

#[test]
fn another_mailbox_cannot_discard_a_message() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let laptop_token = token(&db, "laptop");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let laptop = login(&server, dir.path(), "laptop", &laptop_token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&dell.config, &mem);
    let sent = ok(send(&dell, "dell", "api", "not the laptop's"));
    let id = sent.trim_start_matches("queued as ");

    let refused = stele(
        &[
            "--config",
            laptop.config.to_str().unwrap(),
            "discard",
            "--yes",
            id,
        ],
        &[],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("404"));

    let status = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "status"],
        &[],
    ));
    assert!(status.contains("unread:  1"), "{status}");
}

fn notifier(dir: &Path) -> PathBuf {
    let path = dir.join("notify");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "#!/bin/sh").unwrap();
    writeln!(
        file,
        "printf '%s %s %s %s\\n' \"$STELE_DRAFTS\" \"$STELE_ALREADY\" \"${{STELE_PENDING:-}}\" \"$STELE_MAILBOX\" >> \"$(dirname \"$0\")/notify.log\""
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

fn set_notify(config: &Path, command: &str) {
    let text = std::fs::read_to_string(config).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["notify_cmd"] = serde_json::json!(command);
    std::fs::write(config, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

#[test]
fn a_notifier_runs_only_when_a_draft_is_written() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let notify = notifier(dir.path());
    set_notify(&dell.config, notify.to_str().unwrap());
    let notify_log = dir.path().join("notify.log");

    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");
    ok(pull(&dell, &mem, &log, &[]));
    assert!(!notify_log.exists(), "no drafts, no notification");

    ok(send(&dell, "dell", "api", "one"));
    ok(send(&dell, "dell", "api", "two"));
    // Three drafts already waiting in anathem; the notifier should see them.
    std::fs::write(dir.path().join("mem.log.drafts"), "3\n").unwrap();
    ok(pull(&dell, &mem, &log, &[]));
    assert_eq!(
        std::fs::read_to_string(&notify_log).unwrap(),
        "2 0 3 dell\n",
        "the notifier gets the new count, the retries, the local total and the mailbox"
    );

    // A message anathem refuses writes no draft, so it does not notify.
    ok(send(&dell, "dell", "api", "POISON: refused"));
    let failing = fake_mem_failing_on(dir.path(), "POISON");
    let failed = pull(&dell, &failing, &log, &[]);
    assert!(!failed.status.success());
    assert_eq!(
        std::fs::read_to_string(&notify_log)
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn status_counts_local_drafts_and_survives_a_missing_mem() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&dell.config, &mem);
    let log = dir.path().join("mem.log");
    std::fs::write(dir.path().join("mem.log.drafts"), "3\n").unwrap();

    let status = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "status"],
        &[("MEM_LOG", &log)],
    ));
    assert!(status.contains("unread:  0"), "{status}");
    assert!(status.contains("local:   3 drafts waiting"), "{status}");

    // A missing mem must not take the server side of status down.
    set_mem_bin(&dell.config, Path::new("/nonexistent/stele-mem"));
    let status = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "status"],
        &[],
    ));
    assert!(status.contains("unread:  0"), "{status}");
    assert!(!status.contains("local:"), "{status}");
}

#[test]
fn pull_tells_a_new_draft_from_a_retry() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let sent = ok(send(&dell, "dell", "api", "retry me"));
    assert!(sent.starts_with("queued as "));
    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");

    let first = ok(pull(&dell, &mem, &log, &[]));
    assert!(first.contains("draft "), "{first}");

    // Simulate the lost acknowledgement the retry exists for.
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute("update messages set delivered_at = null", [])
        .unwrap();
    drop(conn);

    let second = ok(pull(&dell, &mem, &log, &[]));
    assert!(second.contains("already here"), "{second}");
    assert!(!second.contains("draft "), "{second}");
    assert!(second.contains("(memory 1)"), "{second}");
}

#[test]
fn exit_codes_separate_unreachable_from_refused() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);

    let out = stele(&["--config", dell.config.to_str().unwrap(), "status"], &[]);
    assert_eq!(out.status.code(), Some(0));

    // The server is there and says no: retrying the same token will not help.
    set_token(&dell.config, "not-a-token");
    let out = stele(&["--config", dell.config.to_str().unwrap(), "status"], &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The server is gone: try again later.
    set_token(&dell.config, &dell_token);
    let config = dell.config.clone();
    drop(server);
    let out = stele(&["--config", config.to_str().unwrap(), "status"], &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn status_json_is_one_document_and_says_when_local_is_unknown() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&dell.config, &mem);
    let log = dir.path().join("mem.log");
    ok(send(&dell, "dell", "api", "hello"));
    std::fs::write(dir.path().join("mem.log.drafts"), "2\n").unwrap();

    let text = ok(stele(
        &[
            "--config",
            dell.config.to_str().unwrap(),
            "status",
            "--json",
        ],
        &[("MEM_LOG", &log)],
    ));
    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["mailbox"], "dell");
    assert_eq!(document["unread"], 1);
    assert_eq!(document["local"], 2);
    assert!(
        document["server"].as_str().unwrap().contains("127.0.0.1"),
        "{text}"
    );

    // anathem cannot answer: the field is null, not a missing line.
    set_mem_bin(&dell.config, Path::new("/nonexistent/stele-mem"));
    let text = ok(stele(
        &[
            "--config",
            dell.config.to_str().unwrap(),
            "status",
            "--json",
        ],
        &[],
    ));
    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(document["local"].is_null(), "{text}");
}

#[test]
fn doctor_reports_a_healthy_mailbox() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&dell.config, &mem);
    let log = dir.path().join("mem.log");

    let stdout = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "doctor"],
        &[("MEM_LOG", &log)],
    ));
    for expected in ["config", "server", "mem", "1.2.0", "drafts", "0 waiting"] {
        assert!(stdout.contains(expected), "{stdout}");
    }
    assert!(!stdout.contains("fail"), "{stdout}");
}

#[test]
fn doctor_fails_without_a_config_and_says_what_to_run() {
    let dir = tempdir();
    let missing = dir.path().join("no-such-config.json");
    let out = stele(&["--config", missing.to_str().unwrap(), "doctor"], &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("config"), "{stdout}");
    assert!(stdout.contains("stele login"), "{stdout}");
}

#[test]
fn doctor_refuses_an_old_mem_and_skips_the_rest_of_anathem() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let old = fake_mem_version(dir.path(), "mem-old", "1.1.0");
    set_mem_bin(&dell.config, &old);

    let out = stele(&["--config", dell.config.to_str().unwrap(), "doctor"], &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1.1.0"), "{stdout}");
    assert!(stdout.contains("1.2.0"), "{stdout}");
    assert!(
        !stdout.contains("drafts"),
        "a mem that is too old should not be asked for drafts: {stdout}"
    );
}

fn request(
    server: &Server,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&[u8]>,
) -> (u16, Vec<u8>) {
    let endpoint = Endpoint::parse(&server.url).unwrap();
    call(&endpoint, method, path, token, body).unwrap()
}

#[test]
fn a_reused_client_id_cannot_carry_another_message() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let laptop_token = token(&db, "laptop");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let laptop = login(&server, dir.path(), "laptop", &laptop_token);

    let first = ok(stele(
        &[
            "--config",
            dell.config.to_str().unwrap(),
            "send",
            "--to",
            "dell",
            "--project",
            "api",
            "--client-id",
            "shared",
            "one",
        ],
        &[],
    ));
    assert!(first.starts_with("queued as "), "{first}");

    // Same id, different recipient: the old answer would claim success and
    // the note would never exist.
    let clash = stele(
        &[
            "--config",
            dell.config.to_str().unwrap(),
            "send",
            "--to",
            "laptop",
            "--project",
            "api",
            "--client-id",
            "shared",
            "two",
        ],
        &[],
    );
    assert_eq!(clash.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&clash.stderr);
    assert!(stderr.contains("409"), "{stderr}");

    let mem = fake_mem(dir.path());
    let log = dir.path().join("mem.log");
    let pulled = ok(pull(&laptop, &mem, &log, &[]));
    assert!(
        pulled.is_empty(),
        "the laptop got a message it should not: {pulled}"
    );
}

#[test]
fn login_keeps_the_optional_config_fields() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&dell.config, &mem);
    set_notify(&dell.config, "true");

    let rotated = ok(stele(
        &["--db", db.to_str().unwrap(), "token", "rotate", "dell"],
        &[],
    ));
    let again = login(&server, dir.path(), "dell", &rotated);

    let text = std::fs::read_to_string(&again.config).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["mem_bin"], mem.to_str().unwrap(), "{text}");
    assert_eq!(value["notify_cmd"], "true", "{text}");
}

#[test]
fn pull_reports_an_unreachable_server_with_exit_two() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let config = dell.config.clone();
    let mem = fake_mem(dir.path());
    drop(server);

    let out = stele(
        &[
            "--config",
            config.to_str().unwrap(),
            "pull",
            "--mem",
            mem.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot connect"), "{stderr}");
}

#[test]
fn a_body_at_the_cap_survives_json_escaping() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);

    // Control characters become six bytes each inside the JSON envelope;
    // the request cap has to be sized for that.
    let body = "\u{1}".repeat(stele::api::MAX_BODY);
    let sent = ok(send(&dell, "dell", "api", &body));
    assert!(sent.starts_with("queued as "), "{sent}");
}

#[test]
fn the_queue_cap_answers_429_and_health_is_public() {
    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let _dell = login(&server, dir.path(), "dell", &dell_token);

    for i in 0..stele::api::MAX_PENDING {
        let payload = serde_json::json!({
            "client_id": format!("cap{i}"),
            "to": "dell",
            "project": "api",
            "body": "x",
        });
        let raw = serde_json::to_vec(&payload).unwrap();
        let (status, body) = request(&server, "POST", "/messages", Some(&dell_token), Some(&raw));
        assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
    }
    let payload = serde_json::json!({
        "client_id": "over",
        "to": "dell",
        "project": "api",
        "body": "x",
    });
    let raw = serde_json::to_vec(&payload).unwrap();
    let (status, _) = request(&server, "POST", "/messages", Some(&dell_token), Some(&raw));
    assert_eq!(status, 429);

    let (status, body) = request(&server, "GET", "/health", None, None);
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("\"ok\":true"));
    let (status, _) = request(&server, "POST", "/health", None, Some(b""));
    assert_eq!(status, 405);
}

#[cfg(unix)]
#[test]
fn doctor_warns_about_a_readable_config_without_failing() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir();
    let db = dir.path().join("stele.db");
    let dell_token = token(&db, "dell");
    let server = start(&db);
    let dell = login(&server, dir.path(), "dell", &dell_token);
    let mem = fake_mem(dir.path());
    set_mem_bin(&dell.config, &mem);
    let log = dir.path().join("mem.log");
    std::fs::set_permissions(&dell.config, std::fs::Permissions::from_mode(0o644)).unwrap();

    let stdout = ok(stele(
        &["--config", dell.config.to_str().unwrap(), "doctor"],
        &[("MEM_LOG", &log)],
    ));
    assert!(stdout.contains("warn"), "{stdout}");
    assert!(stdout.contains("chmod 600"), "{stdout}");
}
