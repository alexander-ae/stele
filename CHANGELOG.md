# Changelog

## Unreleased

- `pull` reads `mem save --json`: it reports a new draft separately from an
  `--ext-id` that was already here, and the notifier gets `STELE_DRAFTS` (new),
  `STELE_ALREADY` (retries) and `STELE_PENDING` (drafts waiting after the pull).
- `status` shows `unread` on the server and the local drafts waiting, when
  anathem can answer; a missing `mem` only drops the local line.
- The minimum anathem is now 1.2, for the `--json` documents.

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
