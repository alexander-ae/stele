# Installation and configuration

This is the reference for putting stele on a machine. The systemd units and
their caveats live in [`deploy/`](../deploy/), the threat model in
[security.md](security.md), and the protocol in [design.md](design.md).

stele is two things: a small server on one machine that is always on, and a
client on every machine that sends or receives. It stores messages in SQLite
and hands each one to anathem as a draft; it does not keep memory of its own.

## Requirements

- **Rust 1.89 or newer** only to build from source (built and tested with
  1.99). The prebuilt binary needs no Rust. `Cargo.toml` is the source of
  truth for the minimum.
- **A prebuilt platform**: Linux x86_64 (glibc 2.35 or newer) or macOS
  arm64/x86_64. Other platforms can use `--build` with Rust and a C compiler.
- **anathem 0.10 or newer** on every receiving machine: `pull` passes
  `mem save --external --ext-id`, which older versions do not know. anathem
  1.0 adds the greeting count that says drafts are waiting.
- **A Unix-like host** for the `sh -c` used by `notify_cmd` and for the
  systemd examples. The core is portable; the deployment examples are not
  Windows.
- A **private network** between the machines: a tailnet, a VPN, a LAN, or an
  SSH tunnel. The binary speaks plain HTTP and refuses `https://`; see
  [Transport](#transport).

## Install a release (recommended)

```bash
sh scripts/install.sh
# or, without cloning:
curl -fsSL https://gitlab.com/__alexander__/stele/-/raw/main/scripts/install.sh | sh
```

The script detects the platform, downloads `stele-<platform>.tar.gz` from the
latest GitLab release, checks its `.sha256`, and installs the binary in
`~/.local/bin`. It does not create tokens, services or config.

| flag | meaning |
|---|---|
| `--tag v0.1.0` | install that release instead of the latest. |
| `--prefix <dir>` | install somewhere other than `~/.local/bin`. |
| `--build` | compile from source instead of downloading. |
| `--no-verify` | skip the checksum (not recommended). |

The first release is checked with `sha256`; a minisign signature comes in a
later release. If the platform has no prebuilt binary, use `--build` with Rust
and a C compiler installed.

## Build from source

```bash
git clone https://gitlab.com/__alexander__/stele.git
cd stele
make check     # fmt --check, clippy -D warnings, tests
make build     # cargo build --release
install -m 755 target/release/stele ~/.local/bin/stele
```

`make` puts `~/.cargo/bin` on `PATH`; rustup was installed with
`--no-modify-path`, so a fresh shell may not have `cargo` otherwise.

The binary is about 2.8 MB, stripped, and `rusqlite` compiles SQLite in, so
there is no `libsqlite3-dev` to match against the host.

## The mailbox machine

Pick one machine that is always on. Everything below runs there.

```bash
# one token per machine; each token is printed once, on stdout
stele --db ~/.local/share/stele/stele.db token add dell
stele --db ~/.local/share/stele/stele.db token add laptop

# bind a private address; 127.0.0.1 is the default
stele --db ~/.local/share/stele/stele.db serve --bind <private-ip>:3900
```

The server prints `listening on http://<address>` on start. `stele serve` is
the only long-running process; it is one thread and one SQLite connection.

Install it as a systemd user service:

```bash
mkdir -p ~/.config/systemd/user
cp deploy/stele-serve.service ~/.config/systemd/user/
# edit ExecStart: the bind address and the database path
systemctl --user daemon-reload
systemctl --user enable --now stele-serve
systemctl --user status stele-serve
```

The unit ships with `10.0.0.10:3900` as a placeholder. If the address only
exists after a VPN, add the VPN's unit to `After=` (the file has the
commented example). If the service stops when the session closes, enable
lingering once:

```bash
loginctl enable-linger "$USER"
```

Check it from another machine that can reach the address:

```bash
curl -s http://<private-ip>:3900/health   # {"ok":true}
```

## Every client machine

```bash
stele login --url http://<private-ip>:3900 --token -
# paste the token from `token add`, then Ctrl-D
stele whoami     # prints the mailbox name
stele status     # server, mailbox and pending count
```

`login` checks the token with `GET /me` before writing
`~/.config/stele/config.json` (mode `0600`). Use `--token -` so the token does
not appear in the process list.

## Receiving

On each machine that must archive messages:

```bash
cp deploy/stele-pull.service deploy/stele-pull.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now stele-pull.timer
systemctl --user list-timers stele-pull.timer
```

The timer runs every five minutes. The unit sets
`PATH=~/.local/bin:...` because systemd does not read your shell profile and
`mem` is not otherwise found.

A draft is not searchable. Read and accept it:

```bash
mem recent --all --external
mem show 42
mem accept 42
```

`--all` matters: `mem recent` defaults to the project of the directory you are
in, and the sender chose the project. anathem 0.10 also says when mail is
filed under another project.

A note that anathem keeps refusing can be acknowledged without a draft:

```bash
stele discard 42         # prints sender, project and the first lines; asks
stele discard --yes 42   # no prompt
```

## Configuration reference

There is no server config file. The client config is one JSON file.

### Client config

`~/.config/stele/config.json`, mode `0600`. `login` writes the first three
keys; the rest are optional.

| key | required | meaning |
|---|---|---|
| `url` | yes | `http://host:port` of the mailbox. `https://` is refused. |
| `mailbox` | yes | this machine's name, as returned by `GET /me`. |
| `token` | yes | the secret from `stele token add`. |
| `mem_bin` | no | path to `mem` when it is not on `PATH`. |
| `mem_db` | no | anathem database for `pull`; the default is `mem`'s. |
| `notify_cmd` | no | shell command to run after a pull writes a draft. |

```json
{
  "url": "http://10.0.0.10:3900",
  "mailbox": "laptop",
  "token": "…",
  "mem_bin": "/home/alexander/.local/bin/mem",
  "mem_db": "/home/alexander/.anathem/mem.db",
  "notify_cmd": "ntfy publish stele \"$STELE_DRAFTS draft(s) in $STELE_MAILBOX\""
}
```

Resolution order for the file: `$STELE_CONFIG`, then
`$XDG_CONFIG_HOME/stele/config.json`, then `~/.config/stele/config.json`.
`stele login` writes the same path.

### Environment

| variable | used by | meaning |
|---|---|---|
| `STELE_CONFIG` | client | config file path. |
| `STELE_DB` | `serve`, `token` | server database path. |
| `STELE_DRAFTS` | `notify_cmd` | number of drafts just written. |
| `STELE_MAILBOX` | `notify_cmd` | mailbox that received them. |

Defaults for the server database: `$XDG_DATA_HOME/stele/stele.db`, then
`~/.local/share/stele/stele.db`.

### Server flags

| command | flag | default | meaning |
|---|---|---|---|
| `serve` | `--bind ADDR` | `127.0.0.1:3900` | address to listen on. |
| `serve` | `--db PATH` | see above | SQLite file. |
| `serve` | `--verbose` | off | log one line per request to stderr. |
| `token add\|rotate\|list\|remove` | `--db PATH` | see above | same database. |

`--db` and `--config` are global and go before the subcommand:
`stele --db … serve`, `stele --config … pull`.

### Commands

| command | what it does |
|---|---|
| `login --url URL --token TOKEN\|-` | validates the token and writes the config. |
| `whoami` | the mailbox this machine is. |
| `status` | server, mailbox, pending count. |
| `peers` | every mailbox the server knows. |
| `send --to M --project P [BODY\|-]` | queues one message; retries a transport failure with the same id. |
| `pull [--dry-run] [--limit N] [--mem PATH] [--mem-db PATH]` | fetches unread and writes drafts; default limit 50, max 200. |
| `discard [--yes] ID` | acknowledges one pending message without a draft. |
| `serve` | the mailbox. |
| `token add\|rotate\|list\|remove` | mailbox and token management. |

### Fixed limits

These are constants in `src/api.rs`, not configuration: the mailbox is meant to
stay small and predictable.

| limit | value |
|---|---|
| message body | 32 KB |
| HTTP request (JSON envelope included) | 64 KB |
| unacknowledged messages per recipient | 200 |
| message lifetime | 7 days |
| one inbox read | about 1 MB of bodies |
| mailbox, project, client id | 64 bytes |

Names: mailbox allows letters, digits and `. _ - @`; `client_id` adds `:`.
Project names follow anathem — any non-empty string without control
characters. A value outside a charset is a `400`.

## Transport

stele is network-agnostic: it opens a TCP connection to `http://host:port`.
Use a private address — tailnet, VPN, LAN — or an SSH tunnel:

```bash
ssh -N -L 3900:127.0.0.1:3900 <mailbox-host>
# then --url http://127.0.0.1:3900
```

TLS is not implemented and `https://` is refused on purpose. On a network you
do not control, wrap the port (VPN, tailnet, SSH) instead of exposing it.

## Upgrades, backups and removal

- **Upgrade:** `sh scripts/install.sh` installs the latest release; then
  restart `stele-serve` on the mailbox machine. The schema is created with
  `create table if not exists` and has not changed, so an existing database
  keeps working. A future schema change needs a migration; there is none yet.
- **Backup:** `sqlite3 ~/.local/share/stele/stele.db ".backup backup.db"`, or
  copy the file with the service stopped. anathem is backed up separately.
- **Rotate:** `stele --db … token rotate laptop`, then `stele login` again on
  that machine. `token remove` also deletes its undelivered mail.
- **Remove:** `systemctl --user disable --now stele-serve stele-pull.timer`,
  delete the binary, `~/.config/stele/` and, if you want, the server database.
  The anathem drafts are unaffected.

## Troubleshooting

| symptom | cause and fix |
|---|---|
| `HTTP 401` | wrong or rotated token: `stele login` again. |
| `HTTP 413` | body over 32 KB (the JSON envelope makes the request slightly larger; the request cap is 64 KB). |
| `HTTP 429` | the recipient has 200 unread: `stele status`, then `pull` or `discard`. |
| `there is no mailbox "x"` | `stele --db … token add x` on the server, or `stele peers`. |
| `cannot run "mem"` | `mem` is not on `PATH`; set `mem_bin` or fix the unit's `PATH`. |
| `stele needs anathem 0.10 or newer` | upgrade `mem` on the receiving machine. |
| `response … is too large` | an old client before the byte-batched inbox; upgrade stele. |
| `stele speaks plain HTTP` | replace `https://` with `http://` over a private network or tunnel. |
| `discard` returns 404 | the id is not a pending message for this mailbox. |

## See also

- [design.md](design.md) — protocol, storage, delivery and acknowledgements.
- [security.md](security.md) — threats and the limits of the draft gate.
- [handoff.md](handoff.md) — what a useful note contains.
- [benchmarks.md](benchmarks.md) — measured footprint and dependency choices.
- [`deploy/`](../deploy/) — the systemd units and their notes.
