# Prior art: what exists, and what stele borrows

This document exists so the next person does not have to rediscover the
landscape. It separates two problems that are easy to blur:

- **Memory**: what an agent knows, and how it is retrieved. That is anathem's
  job, and this repository does not duplicate it.
- **Delivery**: how a note reaches an agent that is not running, on another
  machine, and becomes something a person reviews. That is stele's job.

No product found does both with the same shape: a self-hosted mailbox that
turns each message into an anathem draft and keeps the human gate.

## Tools

| project | what it is | worth borrowing | not worth borrowing |
|---|---|---|---|
| [AgentMail](https://www.agentmail.to/) | Hosted email-inbox API for agents: one address per agent, threads, attachments, webhooks/WebSockets, custom domains, allow/block lists, MCP, IMAP/SMTP. Free tier: 3 inboxes, 3,000 emails/month. | Arrival notification without content, per-agent identity, an allowlist. | Threads, attachments, custom domains, cloud, an MCP server. |
| [ATP](https://arxiv.org/abs/2610.38398) (arXiv 2610.38398) | Server-mediated, cross-organization agent-to-agent protocol: stable logical addresses, the receiving domain stores while the agent is offline, signed envelopes, source verification after delayed reading. | Validation of the architecture; a signed envelope *if* stele ever leaves one administrative domain. | Federation between domains today. |
| [Supermemory](https://supermemory.ai/) | Memory and context API: namespaces, profiles, hybrid search, connectors, processing states (queued/extracting/indexing/done), exact-ID forgetting. | Namespace scoping, explicit processing states, exact-ID deletion. | Connectors, graph extraction, hosted profiles. |
| [Mem0](https://mem0.ai/) | Memory layer: `user_id`/`agent_id`/`run_id` scopes, a shared vector collection with metadata filters, hosted and OSS. Its coding-agent experiment shares memory across repos, but the receiver must **search**; nothing is delivered. | The scoping model, and the lesson from CVE-2026-116575: do not concatenate identifiers into an unescaped composite key. | LLM extraction, graph memory, platform. |
| [Letta](https://docs.letta.com/concepts/shared-memory) | Stateful agents with shared memory blocks and shared memory repositories. The docs describe coordination “without explicit agent-to-agent messaging”. | Read-only vs read-write shared scopes. | Shared state as a substitute for delivery to an offline machine. |
| [Concord](https://github.com/Get-Concord-AI/concord-mcp) | Local-first SQLite mailbox for agents, with hooks and receipts. | Receipts; hook integration. | A second daemon with npm and telemetry. |
| [A2A](https://a2a-protocol.org/) | Agent-to-agent protocol: Agent Card, task lifecycle, push notifications. | `peers` (already a minimal Agent Card) and the push-notification idea. | The task state machine; stele is fire-and-forget. |
| SQS / Temporal | Queue patterns: acknowledgement, visibility timeout, dead-letter queue, receipt handles, retention. | Explicit attempts / dead-letter / discard for a poison message. | The infrastructure and the operational weight. |
| [anathem](https://gitlab.com/__alexander__/anathem) | Local memory for coding agents: SQLite, BM25 plus an optional vector layer, MCP, drafts from another machine, `mem accept`. | The draft rule, provenance, the greeting count. | Turning it into a network service. |

## What the papers say

| paper | finding | what it means for stele |
|---|---|---|
| [2610.38398](https://arxiv.org/abs/2610.38398) ATP | A server-mediated design with stable addresses, offline storage and signed envelopes supports long-lived cross-organization collaboration. | The architecture is sound. Signing is a federation feature, not a single-owner one. |
| [2610.36523](https://arxiv.org/abs/2610.36523) HSC-Bench | Structured handoff state improves evidence recovery in every model and continuation in most, over 1,039 tasks. | A handoff recipe (what changed, evidence, what is left) is worth more than raw prose. |
| [2608.24569](https://arxiv.org/abs/2608.24569) | Constraints erode across handoffs: ownership deferral deactivates 76.7% of blockers, multihop compression 97.2%. | A note is not a contract. Do not send permissions or “musts” as prose; the receiver re-verifies. |
| [2609.36855](https://arxiv.org/abs/2609.36855) | An incorrect upstream message flips a correct answer in up to 32% of cases; 94% of harmful cases copy the upstream answer. | Provenance and review matter more than fluent messages. |
| [2610.33411](https://arxiv.org/abs/2610.33411) | Agent-to-agent “social engineering” works because identity, evidence and permission are separate judgments. | The token gives identity, not authority. A message cannot grant permission. |
| [2610.39109](https://arxiv.org/abs/2610.39109) PCM | A2A and MCP authenticate the channel, not the sender's claims; proof-carrying messages attach evidence. | Signed claims only if stele federates. Inside one owner's network, the server is the trust anchor. |
| [2610.35835](https://arxiv.org/abs/2610.35835) I-HITL | Human approval is only inalienable when it sits outside the agent's reach; agents can harvest OTPs. | The draft gate is a **review** gate: an agent with a shell can run `mem accept`. Say so plainly. |
| [2610.31355](https://arxiv.org/abs/2610.31355) / [2611.12412](https://arxiv.org/abs/2611.12412) | Asynchronous shared buffers help; a single “check your workspace again” changes whether an agent looks. | Anathem 1.0's draft count in the greeting is exactly that nudge. |
| [2610.32429](https://arxiv.org/abs/2610.32429) Async-RBench | Asynchronous agents fail most on duplicate reports and subagent failures. | Idempotency (`--ext-id`) and continue-on-error are the right direction. |
| [2610.41056](https://arxiv.org/abs/2610.41056) / [2611.06545](https://arxiv.org/abs/2611.06545) | Memory robustness means rejecting poisoned input without freezing on outdated facts; summaries can become unsupported instructions. | Keep the draft as data, label where it came from, never auto-accept. |

## What stele deliberately does not borrow

- **MCP server.** It would add tools to every prompt for something the shell
  already does, and put stele between the model and anathem.
- **Threads and attachments.** The mailbox is not a chat; 32 KB of text and a
  project name are enough to hand off a fact.
- **Cloud, connectors, profiles, graph memory, LLM extraction.** That is a
  memory product; anathem already covers memory, and extraction by LLM was
  measured as noise.
- **Shared memory across machines.** Letta and Supermemory share state; stele
  delivers a note to a machine that is offline and lets local memory decide
  what to keep.
- **Federation and signed envelopes.** One owner, one private network. ATP and PCM are
  the reference if that ever changes.

## What is worth adding later

Small and measured. The threat-model document, identifier validation, `status`,
`discard` and a content-free arrival notification are now in. They were the
whole shortlist; each one is a separate issue and none of them adds a
dependency.

## Sources

- AgentMail: <https://www.agentmail.to/> and its public docs.
- ATP: <https://arxiv.org/abs/2610.38398>.
- Supermemory: <https://supermemory.ai/> and the public API docs.
- Mem0: <https://mem0.ai/>; CVE-2026-116575 on scope-key construction.
- Letta shared memory: <https://docs.letta.com/concepts/shared-memory>.
- Concord: <https://github.com/Get-Concord-AI/concord-mcp>.
- A2A: <https://a2a-protocol.org/>.
- Paper ids are linked in the table; all are 2025–2026 arXiv preprints.
