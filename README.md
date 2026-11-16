# stele

A *stele* is a stone slab with an inscription, set upright where whoever comes
next will find it. This one carries a message from one coding agent to another
and leaves it in [anathem](https://gitlab.com/__alexander__/anathem) as a draft.

**Status: v0.2.0, working.** One binary, one SQLite file, no async runtime.

The source, issues and merge requests live on
[GitLab](https://gitlab.com/__alexander__/stele). The
[GitHub repository](https://github.com/alexander-ae/stele) is a mirror that
builds the binaries, nothing else: its issues are off and a merge request
belongs on GitLab.

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

Prebuilt binary — Linux x86_64 (glibc 2.34+) and macOS arm64/x86_64:

```bash
sh scripts/install.sh
```

It downloads the latest GitLab release, checks its `sha256` and installs it in
`~/.local/bin`. Without the repository:

```bash
curl -fsSL https://gitlab.com/__alexander__/stele/-/raw/main/scripts/install.sh | sh
```

`--tag v0.2.0`, `--prefix <dir>`, `--build`, `--verify-signature` and
`--no-verify` are described in
[`docs/installation.md`](docs/installation.md). Releases are signed with
minisign.

### Checking a release

Public key (ID `46EB3322796C3BBF`):

```
RWS/O2x5IjPrRqB3Ie40VlsV7wpp5AwR6cs9PxlokY7zYK6MN4FZoS50
```

```bash
sh scripts/install.sh --verify-signature
# or by hand, after downloading an asset and its .minisig:
minisign -V -p stele.pub -m stele-linux-x86_64.tar.gz
```

The key is also in [`stele.pub`](stele.pub) and in the GitLab project
description. The description is the copy a repository tampered with on its own
cannot change.

From source, with Rust 1.89 or newer (built with 1.99):

```bash
cargo build --release
install -m 755 target/release/stele ~/.local/bin/stele
```

The full guide — requirements, server and client setup, every config key,
limits, upgrades and troubleshooting — is
[`docs/installation.md`](docs/installation.md).

The binary is about 2.9 MB, stripped. `rusqlite` compiles SQLite in, so there
is no `libsqlite3-dev` to install and nothing to match against the system.

The receiving side needs **anathem 1.2 or newer**: `stele pull` hands each
body to `mem save --external --json`, which says whether it wrote a new draft
or found the `--ext-id` already there, and `stele status` counts the drafts
waiting with `mem recent --json`. An older `mem` does not know `--json`; the
pull stops before acknowledging rather than guessing. anathem also announces
waiting drafts in the MCP greeting, which is what keeps the mailbox from going
unnoticed.

## Checks

```bash
make check   # fmt, clippy and tests
make build   # the release binary
```

The same checks run in GitHub Actions on every push and pull request, on Linux
and macOS. GitLab mirrors every branch to `alexander-ae/stele`, so a branch is
tested whether it is opened as a GitLab merge request or not.

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
stele status --json
stele doctor
stele pull --dry-run
stele pull
stele discard --yes 42
mem recent --all --external
mem accept 42
```

`doctor` checks the config, the server and its token, `mem` (1.2 or newer),
the drafts waiting and the pull timer, and exits 1 if something is broken.
It never changes anything.

`status --json` prints `{"server":…,"mailbox":…,"unread":…,"local":…}`, with
`local` null when anathem cannot answer.

`discard` acknowledges one message **without writing a draft**; it asks first
unless `--yes`. Use it only for a note anathem keeps refusing: the row stays
until its TTL, but the note is gone.

`pull` can also notify a person through `notify_cmd` in the client config; it
sees `STELE_DRAFTS` (new), `STELE_ALREADY` (retries), `STELE_PENDING` (drafts
waiting locally) and the mailbox, never a message. See
[`deploy/README.md`](deploy/README.md#notifying-a-person).

`mem recent` defaults to the project derived from the directory you are
standing in, and a message may have been filed under another one. `--all`
shows everything, drafts included; anathem also says so when mail is filed
under another project. anathem 1.0 goes one step further:
the MCP greeting says how many received memories are waiting (a count and the
command, never their content), so the agent knows to look. `stele status`
shows the unread messages on the server and the drafts waiting locally.
`mem show 42` reads one; `mem accept 42` lets it into search. Accepted messages
keep `[from: laptop]` wherever they appear.

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
- [docs/installation.md](docs/installation.md) — build, mailbox, clients,
  every config key, limits and troubleshooting.
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
- **Delivers exactly once on anathem 1.2+.** `pull` passes `--ext-id
  stele:<message id>` and reads `mem save --json`, so a retry after a lost
  acknowledgement writes nothing and is reported as already here, not as a
  new draft. With an older `mem` the message stays in the mailbox; it never
  falls back to a duplicate.

## License

MIT. See [`LICENSE`](LICENSE).
