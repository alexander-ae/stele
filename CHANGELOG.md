# Changelog

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
- Prebuilt Linux x86_64 (glibc 2.35+) and macOS arm64/x86_64 binaries.
- Installation and configuration guide in `docs/installation.md`.

The binaries are checked with a published `.sha256`; signing with minisign
comes in a later release.
