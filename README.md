# stele

A *stele* is a stone slab with an inscription, set upright where whoever comes
next will find it. This one carries a message from one coding agent to another
and leaves it in [anathem](https://gitlab.com/__alexander__/anathem) as a draft.

**Status: v0.1.0, working.** One binary, one SQLite file, no async runtime.

```
stele send --to dell --project api "The API contract changed to v2"
stele pull
stele serve
```

- `send` is explicit: the agent runs it from the shell it already has.
- `pull` fetches what is addressed to this machine and writes each message as
  an external draft, which someone has to accept before it becomes memory.
- `serve` is the mailbox itself, on a machine that is always on.

It adds no MCP tools and no prompt surface: the agent talks through its shell
and remembers through `mem`. If the other side does not run anathem, this is a
mailbox nobody empties.

## Install

Rust 1.89 or newer (built with 1.99).

```bash
cargo build --release
install -m 755 target/release/stele ~/.local/bin/stele
```

The binary is about 2.8 MB, stripped. `rusqlite` compiles SQLite in, so there
is no `libsqlite3-dev` to install and nothing to match against the system.

The receiving side needs **anathem 0.10 or newer**: `stele pull` hands each
body to `mem save --external --ext-id`, so a retried delivery lands in the
draft that already exists. An older `mem` does not know the flag; the pull
stops before acknowledging rather than saving a duplicate. anathem 1.0 also
announces waiting drafts in the MCP greeting, which is what keeps the mailbox
from going unnoticed.

## Checks

```bash
make check   # fmt, clippy and tests
make build   # the release binary
```

## Set it up

On the machine that is always on — the mailbox:

```bash
stele --db ~/.local/share/stele/stele.db token add dell
stele --db ~/.local/share/stele/stele.db token add laptop
stele --db ~/.local/share/stele/stele.db serve --bind <private-ip>:3900
```

Each `token add` prints the token **once**, on stdout, and how to use it on
stderr. The server listens on plain HTTP; bind it to the interface you trust —
a private address (tailnet, VPN, LAN) or `127.0.0.1` behind a tunnel — and do
not expose the port to a network you do not control.

On every machine, once:

```bash
stele login --url http://<mailbox-address>:3900 --token -
# paste the token, then Ctrl-D
stele whoami
```

The config lives in `~/.config/stele/config.json`, mode `0600`.

## Transport

stele is network-agnostic. It opens a TCP connection to `http://host:port` and
speaks HTTP/1.1. The examples use a tailnet because that is the network this
machine is on, not because the binary knows about it: any route that reaches
the port works:

- a tailnet, a WireGuard or other VPN, or a home/lab LAN;
- an SSH tunnel, when the machines have SSH but no shared network:
  `ssh -N -L 3900:127.0.0.1:3900 dell`, then `--url http://127.0.0.1:3900`;
- any other TCP path, if you accept that the traffic is plaintext.

What it does **not** do is TLS. It rejects `https://` on purpose: a TLS stack
would be most of the dependency budget and the binary, for a mailbox that moves
tens of messages a day. On a network you do not control, wrap it (tailnet, VPN,
SSH) instead of exposing the port; on a public address the bearer token and the
message bodies would travel in the clear.

## Use it

```bash
stele send --to dell --project api "The contract changed to v2"
printf 'the whole note\n' | stele send --to dell --project api -
stele peers
stele status
stele pull --dry-run
stele pull
stele discard --yes 42
mem recent --all --external
mem accept 42
```

`discard` acknowledges one message **without writing a draft**; it asks first
unless `--yes`. Use it only for a note anathem keeps refusing: the row stays
until its TTL, but the note is gone.

`pull` can also notify a person through `notify_cmd` in the client config; it
sees a count and the mailbox, never a message. See
[`deploy/README.md`](deploy/README.md#notifying-a-person).

`mem recent` defaults to the project derived from the directory you are
standing in, and a message may have been filed under another one. `--all`
shows everything, drafts included; since anathem 0.10 the command also says so
when mail is filed under another project. anathem 1.0 goes one step further:
the MCP greeting says how many received memories are waiting (a count and the
command, never their content), so the agent knows to look. `mem show 42` reads
one; `mem accept 42` lets it into search. Accepted messages keep
`[from: laptop]` wherever they appear.

A dropped connection during `send` is retried with the same message id, so the
retry does not duplicate. A note that anathem refuses does not stop the batch:
it stays unread and is reported, and the next pull retries only that one.

## How the other agents use it

Any agent with a shell — opencode, Claude Code, Codex, Cursor — uses it with
no per-client configuration. The cheapest integration is one line in the
agent's instruction file:

```
To leave a note for the agent on another machine: stele send --to <mailbox> --project <project> "..."
```

That is roughly 25 tokens per session. Sending is the agent's decision, so it
has to know the command exists. Receiving does not have to be in the prompt:
run `stele pull` from a systemd timer (or a `SessionStart` hook) and the draft
is waiting in anathem the next time somebody looks. A person still accepts it;
that is the point of the design, not a missing feature.

Who is "from" is decided by the token, not by the message, so an agent cannot
sign as another machine. Every agent on one machine shares
`~/.anathem/mem.db`, so a mailbox addresses a machine; the `project` field is
what routes the note to the right work.

## Design

- [docs/design.md](docs/design.md) — transport, storage, why the draft rule
  decides everything else, and what changed from the proposal.
- [docs/prior-art.md](docs/prior-art.md) — what already exists (AgentMail, ATP,
  Supermemory, mem0, Letta, A2A), the recent papers, and what stele borrows.
- [docs/security.md](docs/security.md) — assets, threats, and the limits of the
  draft gate.
- [docs/handoff.md](docs/handoff.md) — what a note the next agent can use
  looks like.
- [docs/benchmarks.md](docs/benchmarks.md) — measured binary size, RSS, idle
  CPU, latency, database growth, and the dependency comparison.
- [deploy/](deploy/) — systemd user units for `serve` and for the pull timer.

## What it deliberately does not do

- **Not a chat.** No threads, presence or typing indicators.
- **Not multiuser.** One token per mailbox, no accounts or permissions. A
  shared workspace for teams is a different product.
- **Does not push.** Delivery is a draft; the greeting says how many are
  waiting and the command, and the content stays behind `mem recent --all
  --external` until a person accepts it.
- **Is not useful alone.** Without `mem` on the receiving side, nothing
  archives what arrives.
- **Does not do TLS.** Use a network that already encrypts (tailnet, VPN, SSH
  tunnel), or put the mailbox behind one; the client refuses `https://`.
- **Delivers exactly once on anathem 0.10+.** `pull` passes `--ext-id
  stele:<message id>`, so a retry after a lost acknowledgement writes nothing:
  the draft that already exists is the one somebody accepts. With an older
  `mem` the flag is rejected and the message stays in the mailbox; it never
  falls back to a second draft.

## License

MIT. See [`LICENSE`](LICENSE).
