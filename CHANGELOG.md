# Changelog

## Unreleased

- `stele doctor` checks the config (and its mode), the server and token,
  `mem` (1.2 or newer), the drafts waiting and `stele-pull.timer`, with a hint
  per failure; it reads and probes only, and exits 1 when a check fails.
- Exit codes a script can act on: `2` when the server cannot be reached, `3`
  when it answered and refused, `4` when it asked to try again later, `1` for
  local failures. Classified at the transport boundary, not by matching
  messages.
- `status --json` prints one document, with `local: null` when anathem cannot
  answer instead of dropping the line.
- A reused `client_id` for another message is now a `409`; before, the new
  message was silently dropped while the sender was told it had been queued.
- The request cap is sized from the worst-case JSON escaping, so a 32 KB body
  always fits; `send` refuses an oversized body before it hits the wire.
- A request now has one deadline for all of its reads: a client dripping a
  byte at a time cannot hold the single-threaded server open indefinitely.
- `stele login` keeps `mem_bin`, `mem_db` and `notify_cmd`; rotating a token
  no longer wipes the notifier.
- `pull` stops at the first unreachable or temporary failure instead of
  waiting out a timeout once per message, and keeps the exit code.
- The notifier has a 30 s deadline; `doctor` probes systemd with a timeout.
- The client config and a new database are created `0600`, not tightened
  after writing.
- `token add|rotate|remove` reject extra arguments; cancelling `discard` exits
  0; a project cannot start with `-`; `sign-release.sh` uploads with `-R`;
  `install.sh --build --tag` is an error rather than a silent no-op.

## v0.2.0 — 2026-11-16

A mailbox that says what it did: which deliveries wrote a draft, which were
retries, and how many drafts are still waiting locally.

- `pull` reads `mem save --json`: it reports a new draft separately from an
  `--ext-id` that was already here, and the notifier gets `STELE_DRAFTS` (new),
  `STELE_ALREADY` (retries) and `STELE_PENDING` (drafts waiting after the pull).
- `status` shows `unread` on the server and the local drafts waiting, when
  anathem can answer; a missing `mem` only drops the local line.
- The minimum anathem is now 1.2, for the `--json` documents.
- GitHub Actions runs `fmt`, `clippy` and the tests on every pushed branch, on
  Linux and macOS (36 s on a cold cache). GitLab mirrors each branch to
  `alexander-ae/stele`, which is what triggers the run.

## v0.1.0 — 2026-11-12

First release. A mailbox between coding agents on different machines, with
anathem as the archive.

- `stele serve`: one blocking accept loop, one SQLite file, bearer tokens
  stored as SHA-256.
- `stele send` / `stele pull`: a message arrives as an anathem draft, never as
  active memory; `--ext-id` makes a retried delivery one draft.
- `stele status`, `stele discard`, `stele peers`, `stele whoami`.
- Optional `notify_cmd` after a pull writes at least one draft, with a count
  and the mailbox and no message content.
- Byte-batched inbox reads, a bounded queue (200 pending, 7-day TTL, 32 KB
  bodies) and a sender that retries a transport failure with the same id.
- Prebuilt Linux x86_64 (glibc 2.34+) and macOS arm64/x86_64 binaries.
- Installation and configuration guide in `docs/installation.md`.

The assets are signed with minisign; the public key (ID `46EB3322796C3BBF`) is
in `stele.pub` and in the GitLab project description. `install.sh
--verify-signature` checks it.
