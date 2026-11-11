# Design

A mailbox between coding agents, on separate machines, with anathem as the
archive. This document is the proposal; it is short on purpose.

## The rule that decides everything else

**A message arrives as a draft, never as active memory.**

`stele pull` writes each message with `mem save - --external --from <sender>
--project <project> --ext-id stele:<message-id>`. That is an anathem feature that already exists: the note
is invisible to search, keeps who sent it, and waits for `mem accept`.

So a remote agent cannot write into your memory. It can leave something you
accept. That is the difference between a mailbox and an open door, and it is
the reason this is a separate program instead of an option inside `mem`.

## The three commands

| command | who runs it | what it does |
|---|---|---|
| `stele send --to <agent> <body>` | the agent, from its shell | posts one message |
| `stele pull` | a timer on the receiving machine | fetches unread, writes drafts, marks delivered |
| `stele serve` | the always-on machine | stores and serves messages |

No MCP server. A second MCP server per agent would add four tools to every
prompt for something the shell already does, and it would put `stele` between
the model and the memory, which is where `mem` belongs.

## Transport

HTTPS REST, JSON. `POST /messages`, `GET /messages?to=<agent>&unread=1`.

Not WebSocket or SSE: messages are tens per day, not thousands per second, and
a socket is state to keep for a latency nobody notices.

TLS is terminated by nginx or by Tailscale; the binary speaks plain HTTP on
loopback. That keeps `rustls` and the certificate renewal out of the code.

Auth: one opaque token per agent, sent in a header, stored as a hash. No
accounts, no OAuth, no registration.

**The token decides who you are, and the body never does.** `from_agent` is
derived server-side from the token presented; a value in the request body is
ignored, not trusted. `GET /messages?to=<agent>` is scoped the same way —
the token picks the mailbox and the parameter cannot widen it.

This is not a detail of the implementation. Everything above rests on
somebody deciding whether to accept a draft, and what they decide on is
`[from: sender]` — the one piece of provenance a received memory carries,
shown by `mem recent --external` and by the greeting, and listed in
anathem's own `docs/security.md` as a mitigation. If any agent holding any
valid token could sign as another, that provenance would be decorative, and
the draft gate would go from "a mailbox, not an open door" to an open door
with a label anyone can write.

One token maps to exactly one agent name, and the mapping lives on the
server. Rotating a token keeps the name; reusing a name requires revoking
the old token first.

## Storage

One SQLite file on the server, same pragmas as anathem (WAL, busy timeout):

```
messages(id, from_agent, to_agent, project, body, created, expires, delivered_at)
```

- TTL of 7 days by default. **Expiry runs on `serve`'s own clock, not on
  `pull`**: driven by traffic instead, a recipient whose machine is off
  accumulates messages that never expire, and the queue cap below then starts
  refusing *new* messages while the stale ones keep their place — the queue
  filling with what nobody wanted and rejecting what somebody just sent.
- Body capped at 32 KB, queue capped at 200 pending per recipient. Without
  both, one agent stuck in a loop fills the disk. The cap is what the TTL
  falls back to, which is why the two are decided together.
- `delivered_at` is written when the message is handed over, not when a person
  reads it — the same honesty as `followed_up` in anathem: record what can be
  observed, not what is assumed.

**`pull` acknowledges only after `mem` exits zero.** That order is what
chooses the failure mode, so it is written here rather than left to whoever
implements it:

- acknowledge **after** the save succeeds → a crash in the window costs a
  duplicate draft, which `mem save --external --ext-id` then collapses into
  one;
- acknowledge **before** → a crash costs the message, with nothing left to
  retry from.

A duplicate is recoverable and a loss is not.

## Resource targets

Light on purpose: this should cost less than anathem, which reaches 64 MB when
it loads its pack.

| | target |
|---|---|
| binary, stripped | 2.5–3 MB |
| RSS, idle | ~6 MB |
| CPU, idle | 0% (blocking accept) |
| local latency | < 1 ms |
| database, 10k messages | ~200 KB |

## Dependencies

```
tiny_http        blocking, one thread per connection
rusqlite         the same choice anathem made
serde, serde_json
chrono
anyhow
```

No `tokio`: a reactor, a timer wheel and a scheduler are a truck engine for a
letter. A blocking server sleeps in `accept()` and costs nothing while idle.

## What it deliberately does not do

- **Not a chat.** No threads, no presence, no typing indicators.
- **Not multiuser.** One token per agent, no accounts, no permissions. A shared
  workspace for teams is a different product built beside this one.
- **Does not push.** Delivery is a draft; the agent sees it with `mem recent
  --external`. Making it appear in the MCP greeting needs a `messages` table
  inside anathem (~200 lines) and is a separate decision.
- **Is not useful alone.** Without `mem` on the receiving side, nothing
  archives what arrives.

## Open questions

- The name `stele` is taken on crates.io and npm (checked 2026-11-10). The
  repository keeps the name; if it is ever published, the crate would be
  `stele-cli` or similar.
- ~~Whether the greeting delivery is worth the 200 lines in anathem~~ —
  answered on the anathem side: the greeting now says how many received
  memories are waiting, a count and a command and nothing of their content,
  measured at 22 tokens and only when any exist. `mem recent --external` also
  stops reporting an empty window when the mail is filed under another
  project. Both landed in anathem 1.0.
- Whether `pull` on a timer (30 s? 5 min?) or on session start only.
