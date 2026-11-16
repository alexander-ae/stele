# Design

A mailbox between coding agents, on separate machines, with anathem as the
archive. This document describes what is built in `src/` and why.

The landscape it borrows from is in [prior-art.md](prior-art.md), the threat
model in [security.md](security.md), the note format in
[handoff.md](handoff.md), and the setup reference in
[installation.md](installation.md).

## The rule that decides everything else

**A message arrives as a draft, never as active memory.**

`stele pull` writes each body with `mem save - --external --from <sender>
--project <project> --ext-id stele:<message-id>`, an anathem feature that
already exists: the note is invisible to search, keeps who sent it, and waits
for `mem accept`.

A remote agent cannot write into your memory. It can leave something you
accept. That is the difference between a mailbox and an open door, and it is
why this is a separate program instead of an option inside `mem`.

The receiving side needs **anathem 1.2 or newer**, which has
`mem save --external --json` and `recent --json` with `status`. `pull` passes
`ext_id = stele:<message-id>` and reads the JSON: a retried delivery is
reported as already here, not as a new draft, and `status` can count the
drafts still waiting.

## The commands

| command | who runs it | what it does |
|---|---|---|
| `stele send --to <mailbox> --project <p> <body>` | the agent, from its shell | posts one message |
| `stele pull` | a timer on the receiving machine | fetches unread, writes drafts, acknowledges |
| `stele serve` | the always-on machine | stores and serves messages |
| `stele token add\|rotate\|list\|remove` | the mailbox operator, on the server | manages tokens, on the server's SQLite file |
| `stele login` / `whoami` / `peers` / `status` | each client | writes or checks the client config, lists mailboxes, or reports how many messages are waiting |
| `stele pull` (`notify_cmd`) | each client, if configured | runs a local command with a draft count and the mailbox, never a message |
| `stele discard <id>` | the recipient, from its shell | acknowledges one message without writing a draft; asks first |

No MCP server. A second MCP server per agent would add tools to every prompt
for something the shell already does, and it would put `stele` between the
model and the memory, which is where `mem` belongs.

## Transport

stele is not tied to Tailscale or to any particular network. It speaks plain
HTTP/1.1, one request per connection, to whatever address it is given: a
tailnet, a WireGuard or other VPN, a LAN, or an SSH tunnel
(`ssh -N -L 3900:127.0.0.1:3900 <host>`).

TLS is not implemented in the binary, and `https://` is refused on purpose: a
TLS stack would be most of the dependency budget for a mailbox that moves tens
of messages a day. On a network that does not already encrypt the link, wrap
the port in something that does; do not expose it in the clear. If direct
public HTTPS is ever needed, it is a separate decision with its own cost, not
a flag to switch on.

```
GET  /health                -> {"ok":true}                  no token
GET  /me                    -> {"mailbox":"dell"}
GET  /peers                 -> {"mailboxes":["dell","laptop"]}
GET  /status                -> {"mailbox":"dell","pending":3}
POST /messages              -> {"id":"…","duplicate":false}  201, or 200 on a retry
GET  /messages?unread=1&limit=50 -> {"messages":[…]}
GET  /messages/{id}         -> one pending message
POST /messages/{id}/ack     -> 204
POST /messages/{id}/discard -> 204, acknowledged without a draft
```

### Who sent it

Auth is one opaque token per mailbox in `Authorization: Bearer <token>`,
stored only as its SHA-256 in SQLite. The token decides the sender: the JSON
body has no `from` field, and a body that carries one — or `from_agent`, or any
other spelling — is ignored when the request is deserialized, because the type
has nowhere to put it. Accepting a claimed name and rejecting the request are
both defensible; silently honouring it is not, and this does not.

The recipient comes from the token too, not from a query parameter: `GET
/messages` reads only the caller's own inbox, whatever `to` says. One token maps
to one name — `mailboxes.name` is the primary key and `token_hash` is unique, so
`token add` refuses a name that already exists and two tokens cannot share one.
Rotating a token keeps the name; a name is only freed by removing its mailbox.

Mailbox names and `client_id` are a narrow charset on purpose: letters, digits
and `. _ - @` (plus `:` for the client id). They are the identity and the retry
key, and they travel into anathem's `--from` label and the logs. Project names
are not restricted the same way: anathem stores any non-empty string without
control characters, so stele accepts the same and only passes it through. A
value outside a charset is a `400`, never silently rewritten.

There are no accounts, no OAuth and no registration: `stele token add` prints 32
random bytes as hex once, and `token rotate` replaces it.

`POST /messages` takes a `client_id` chosen by the sender. `(from, client_id)`
is unique, so retrying a send after a timeout returns the original id instead
of duplicating the message. `stele send` retries a transport failure — connect,
write, read, timeout — up to three times with that same id; an HTTP answer is
never retried. The server collapses the duplicate; the CLI is the part that has
to keep the key.

## Storage

One SQLite file on the server, same pragmas as anathem (WAL, busy timeout,
`synchronous=NORMAL`), created and chmodded `0600` by the binary:

```
mailboxes(name, token_hash unique, created_at)
messages(id, from_box, to_box, client_id, project, body,
         created_at, expires_at, delivered_at,
         unique(from_box, client_id))
```

Limits, enforced on the server:

- TTL of 7 days; expired rows are deleted on every write and every inbox read.
- Body capped at 32 KB, project and mailbox names at 64 bytes. The HTTP request
  cap is a separate 64 KB, so a body at the documented limit fits inside its
  JSON envelope.
- 200 pending (unacknowledged) messages per recipient; a further send gets
  `429` instead of filling the disk.
- One inbox read returns at most ~1 MB of bodies. The client acknowledges what
  it receives, so a full mailbox drains in several pulls instead of producing a
  response the client would refuse to read.

Expiry does not depend on `pull`. Every request that writes or reads messages
sweeps expired rows, and the pending count in the cap already ignores rows
past `expires_at`. A recipient whose machine is off cannot fill the queue with
stale mail or make the server reject what somebody just sent; at worst the
expired rows sit on disk until the next request touches the database.

`delivered_at` is written when the receiver **acknowledges**, not when the
server hands the message over. The client acknowledges only after `mem` exits
zero. The alternative — marking on hand-over — silently loses a message when
the response is cut, which is the wrong failure for a mailbox whose whole
point is not losing the note.

Delivery is exactly-once as long as the receiving `mem` is anathem 1.2 or
newer: `pull` passes `--ext-id stele:<message id>`, the server's random and
stable id, and a retry after a lost acknowledgement writes nothing because
anathem already holds that identifier. The first attempt's draft is the one
somebody accepts.

`--ext-id` is required, not best-effort. An older `mem` rejects the flag,
`pull` exits without acknowledging and the message stays in the mailbox.
Falling back to a save without the key would reintroduce the duplicate this
exists to prevent; a clear failure beats a silent regression.

`pull` delivers the batch message by message. If `mem` refuses one, that
message stays unread and is reported; the rest are written and acknowledged,
and the command exits 1 with the count. One bad note cannot hold the queue.

## Announcing a draft

The draft rule works because a person looks. A mailbox whose drafts nobody
knows about is a mailbox nobody empties, so anathem 1.0 says in the MCP
greeting how many received memories are waiting, and names the command:

```
3 received memories are waiting for review and cannot be searched until
accepted: `mem recent --all --external`.
```

A count and a command, and not one character more: no title, no sender, no
body. The untrusted text still does not enter the system prompt before
somebody agrees to let it in. It is about 22 tokens, only when at least one
draft exists, and `handoff_items: 0` turns it off with the rest of the
greeting.

`mem recent --external` also stopped reporting an empty window when the mail
is filed under another project, which is the ordinary case when the sender
chooses the project name.

## Resource targets

The point of a separate binary is that the mailbox is cheap enough to leave
running. Measured on the Dell (Ubuntu 26.04, 4 cores), release build with
`lto = "fat"`, `codegen-units = 1`, stripped:

| | target | measured |
|---|---|---|
| binary | 2.5–3 MB | 2.89 MB |
| RSS, idle | ~6 MB | 5.7 MB, one thread |
| CPU, idle | 0% | 0 ticks over 10 s |
| local latency | < 1 ms | send p50 0.24 ms, p99 0.82 ms; ack p50 0.15 ms |
| database, 10k messages | ~200 KB | 3.68 MB with ~200-byte bodies |

The 10k-message target was wrong in the proposal: it ignored the row, the
body and two indexes. 3.7 MB for ten thousand notes is still nothing, and the
caps keep the file bounded, but the number in this document is the measured
one.

For scale, `mem` itself is 8 MB stripped and an idle `mem serve` costs 9–14 MB
of RSS **per agent session**. One `stele serve` for all of them is the cheaper
shape.

## Dependencies

```
httparse         parsing HTTP/1.1 on both sides, no dependencies
rusqlite         the same choice anathem made, SQLite compiled in
serde, serde_json
sha2             token hashes
getrandom        tokens and message ids
lexopt           argument parsing, no dependencies
anyhow
```

No `tokio`, no `axum`, no TLS stack, no date/timezone crate. The comparison
that led here, with the size each crate adds to a stripped binary, is in
[benchmarks.md](benchmarks.md).

## What changed from the proposal

- **`tiny_http` → `httparse`.** The proposal picked a blocking server; measured
  against the same hello-world, `httparse` (plus about 100 lines for accept,
  read and write) is 211 KB smaller and 1.1 MB lighter at idle, and a single
  blocking accept loop means one SQLite connection with no mutex. It also
  removes an unmaintained dependency from a security-sensitive path.
- **An acknowledgement endpoint.** Marking on hand-over loses messages; see
  Storage.
- **`--ext-id` and `--json`.** The proposal left delivery at-least-once and
the flag as an open question; anathem shipped `--ext-id` and later `--json`,
and `pull` uses both. A retried delivery is one draft, and the notifier can
tell new drafts from retries. The mailbox now requires anathem 1.2.
- **The greeting announces drafts.** anathem 1.0 adds a count and the command
  when received memories are waiting (22 tokens, only when there are any), so
  the mailbox no longer depends on somebody remembering to look.
- **No HTTP client crate.** `ureq` without TLS still adds 343 KB; the client
  is the same `httparse` code pointed at a response.
- **No `chrono`.** Formatting a UTC timestamp and keeping epoch seconds is
  25 lines with tests, and `chrono` adds 40 KB plus a timezone stack for
  features this program does not use.
- **`clap` → `lexopt`.** 280 KB in a binary this small, for four subcommands.
- **The queue cap counts unacknowledged messages,** not undelivered ones, so a
  slow reader is protected from a fast sender.
- **The server is single-threaded.** At tens of messages a day a worker pool
  only adds state; a slow client can hold the loop for the 10 s read timeout,
  which is bounded and only reachable from the network the port is bound to.

## What it deliberately does not do

- **Not a chat.** No threads, presence or typing indicators.
- **Not multiuser.** One token per mailbox, no accounts, no permissions. A
  shared workspace for teams is a different product built beside this one.
- **Does not push.** Delivery is a draft; the greeting says how many are
  waiting and the command, and the content stays behind `mem recent --all
  --external` and a person's `mem accept`.
- **Is not useful alone.** Without `mem` on the receiving side, nothing
  archives what arrives.

## Security notes

- **New outbound channel.** `stele send` is a way out for text an injected
  agent already has. Mitigations: the recipient must be a mailbox the server
  knows, the body is capped at 32 KB and the pending queue at 200, so the
  bandwidth is bounded. The mailbox is still not a place to send secrets.
- **Plain text at rest.** Message bodies sit in SQLite unencrypted, protected
  by the file mode (`0600`) and the machine's access control, like anathem's
  database.
- **No auto-accept.** A draft is text another model wrote; nothing puts it in
  a system prompt or in search before a person accepts it. `stele pull
  --accept` does not exist on purpose.
- **Tokens are compared as hashes** and live in one file per client, `0600`.
  A config that others can read is reported when it is loaded.
- **Provenance is structural, not decorative.** The sender is the mailbox the
  token belongs to; a body cannot claim another one, and an inbox is read with
  its own token. Without that, the `[from: …]` a person sees when deciding
  whether to accept a draft would be a label anyone could write.

## Open questions

- **Where the pull runs.** A timer is enough because a person accepts the
  drafts anyway. A `SessionStart` hook would make it immediate, at the cost of
  one agent-specific integration per client.
- **The name.** `stele` is taken on crates.io and npm (checked 2026-11-10).
  The repository keeps the name; if it is ever published, the crate would be
  `stele-cli` or similar.
