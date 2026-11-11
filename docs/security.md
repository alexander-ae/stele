# Security: what stele protects, and what it does not

stele carries text written by one model on one machine to another machine,
where it becomes a draft in anathem. That is two risks in one: a network
surface that did not exist before, and an outbound channel for an agent that
already has one.

This document says what is mitigated, how, and what is not. It follows the
same honesty as anathem's `docs/security.md`: there is no filtering that turns
untrusted text into trusted text.

## Assets

| asset | where it lives | why it matters |
|---|---|---|
| Message bodies | server SQLite, plaintext, `0600` | They can contain repository facts, paths and decisions. |
| Mailbox tokens | hash on the server; the token in the client config, `0600` | A token reads and sends as that mailbox. |
| The anathem database | the receiving machine, `~/.anathem/mem.db` | The drafts land there, next to active memory. |
| The mailbox database | the server, `~/.local/share/stele/stele.db` | Queue contents and token hashes. |

## Threats

| threat | what it can do | what stele does | residual risk |
|---|---|---|---|
| **Network attacker** | Read or modify plain HTTP between machines. | Nothing in the binary: TLS is deliberately absent. The documented deployment binds a private address (tailnet, VPN, LAN), where the link is encrypted or trusted. | A LAN or public bind without a VPN is plaintext. The token and the bodies are exposed. |
| **Token theft** | Read the inbox, send as that mailbox, ack or discard. | Tokens are 32 random bytes, stored as SHA-256; the client config and the database are `0600`; `token rotate` replaces one. | Anyone with the file, or root on the client, has the mailbox. No per-token rate limit yet. |
| **Malicious or compromised peer** | Fill the queue, send misleading text, choose a project name. | The sender is the token's mailbox; 200 pending per recipient; 32 KB body; 7-day TTL; a draft never becomes active memory by itself. | The peer can still send 200 plausible lies. The human review is the only filter. |
| **Compromised agent on the receiver** | Read `~/.anathem/mem.db`, run `mem recent`, `mem accept`, `mem save`, and `stele` itself. | Nothing: same user, same files. | **The draft gate is a review gate, not a security boundary.** See below. |
| **Prompt injection inside a message** | Make the receiving model do something when it reads the draft. | The draft is not in search and not in the greeting. `pull` prints only a first line; the body is read with `mem show`, as a tool result. The count in the greeting contains no message text. After acceptance, the note is labelled `[from: …]` and anathem frames memories as data. | A person can accept a malicious note; a model can follow it. The gate reduces the window, it does not remove the content. |
| **Exfiltration through `stele send`** | Send private text to another mailbox. | The recipient must be a configured mailbox; the body is capped at 32 KB; 200 pending per recipient bounds the bandwidth; the server records who sent what until TTL. | `send` is a channel out. Do not run an agent with secrets and a token it does not need; the caps bound volume, not judgment. |
| **Denial of service** | Open connections and stall; fill the queue; fill the disk. | One blocking accept loop; 10 s read/write timeout; 16 KB headers; 64 KB request; 200 pending; TTL; one SQLite file. | A peer on the private network can hold the loop for up to 10 s per connection. A compromised token can keep a mailbox at its cap. There is no per-sender rate limit. |
| **Storage and backups** | Read a stolen disk or backup. | SQLite file `0600`, directory `0700`; bodies expire after 7 days; acked rows stay until then. | Plaintext at rest. Backups inherit the same exposure. |

## The gate is a review gate

The design says a message arrives as a draft and a person accepts it. That is
true, and it is the most important property here: a remote agent cannot write
into active memory by itself.

It is **not** a guarantee that a person actually looked. An agent with a shell
on the receiving machine can run `mem accept <id>`. Recent work on inalienable
human-in-the-loop (I-HITL, arXiv 2610.35835) shows exactly this failure in
another setting: agents harvested one-time codes to bypass a human approval.
The lesson is structural, not a bug in anathem.

If the approval has to survive a compromised or adversarial agent, it has to
live outside the agent's reach:

- run `mem accept` as a different OS user that owns the database, and give the
  agent a different user that cannot write it;
- or accept on a separate device, with the draft copied there;
- or keep the receiving mailbox on a machine the agent cannot log into.

Anything weaker is a review convention. That is still useful — it catches
mistakes and most injections — but it is not an authorization system.

## Prompt injection

A received note is text another model wrote, possibly while summarising a page
it read. The mitigations are:

1. It is stored as a **draft**: not in `search`, not in the MCP greeting, not
   in the handoff summary.
2. `pull` writes it with `mem save --external`; the body is not interpreted by
   stele.
3. The greeting can say that drafts are waiting; it carries **only a count**,
   never a title, sender or body.
4. After acceptance it is labelled `[from: <mailbox>]` and anathem's framing
   says the block is data, not instructions.

None of these makes the model immune. The research in
[prior-art.md](prior-art.md) is consistent: provenance helps, review helps,
and neither is a filter.

## The outbound channel

Anathem's security document says it has no outbound channel. stele is one, by
design: `send` posts text to another machine. The bounds are structural — the
recipient must exist, the body is capped, the queue is capped, the server keeps
a row — not semantic. A note can still be a lie, and an injected agent can
still use the mailbox to move data it already has.

Consequences for operation:

- Give an agent the token for one mailbox, not a shared one.
- Do not put secrets in a note; the mailbox is plaintext and the draft lands in
  a plaintext database.
- Rotate a token when a machine is retired or a repository is shared.
- Treat the mailbox as the same trust level as the machine's shell, not higher.

## Operational checklist

- Bind the private address, not `0.0.0.0`, and keep the port off the public
  interface.
- `stele token add` per machine; `token rotate` on suspicion; `token remove`
  when a machine is gone (it also deletes its undelivered mail).
- Keep the client config `0600`; stele warns when it is readable by others.
- Back up the server database with `sqlite3 stele.db ".backup …"`; back up
  anathem separately.
- Watch the drafts: `mem recent --all --external`, or anathem 1.0's greeting
  count. A mailbox nobody empties is the failure mode this design has.
- `notify_cmd`, if set, is a shell command from the client config. It runs
  with your user and sees only a count and the mailbox; treat the config as
  code.
- Discard only what you have read. `stele discard <id> --yes` acknowledges one
  message without writing a draft; the row stays until TTL, but the note is
  gone.
- For real approval isolation, run `mem accept` outside the agent's user.

## What stele deliberately does not do

- **TLS in the binary.** Use a VPN, a tailnet or an SSH tunnel. It refuses
  `https://` instead of pretending.
- **Per-message signatures or proof-carrying messages.** One owner, one
  server; the token is the identity. ATP/PCM are the reference if stele ever
  federates.
- **Content filtering or instruction detection.** It does not work; believing
  it does is worse than not having it.
- **Automatic acceptance.** A draft is a decision left to a person.
- **Per-sender rate limits.** The queue cap is per recipient, not per sender;
  a compromised token can keep one mailbox full. This is a known gap, tracked
  as a possible improvement.

## References

- anathem's security notes: `docs/security.md` in the anathem repository.
- I-HITL: <https://arxiv.org/abs/2610.35835>.
- Proof-carrying messages: <https://arxiv.org/abs/2610.39109>.
- Agent-to-agent social engineering: <https://arxiv.org/abs/2610.33411>.
- Handoff summaries and unsupported policy: <https://arxiv.org/abs/2611.06545>.
- Memory poisoning and outdated facts: <https://arxiv.org/abs/2610.41056>.
