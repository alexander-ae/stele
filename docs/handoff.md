# A note the next agent can use

`stele send` carries text, and the receiving side turns it into an anathem
draft. The format is free, but some notes survive the handoff and others turn
into a caveat the next agent ignores. The research in
[prior-art.md](prior-art.md) is blunt about it: structured handoff state
improves continuation, and constraints written as prose erode.

## The shape

```
What changed
  The facts: files, endpoints, contracts, versions, decisions taken.
  “/orders now pages by 50; the field is `statusCode` (2 = PAID).”

Evidence
  The command, test or file that shows it, and when.
  “cargo test --test orders, 2026-11-11, green.”

What is left
  The next concrete step, or “nothing”.

Blockers and decisions
  What cannot proceed, and what was already decided. State it as a fact.
  “The old fixture has to be migrated before the billing tests.”

What not to assume
  The parts the sender did not verify.
```

Keep it under a screen. The body cap is 32 KB; a note that needs more is
usually a document that belongs in the repository, with a link.

## What not to send

- **Secrets, tokens, keys, `.env` contents.** The mailbox stores plaintext and
  the token is the sender's identity.
- **Permissions written as prose.** “You are allowed to deploy” is not a grant;
  the receiver checks the repository, the user and the policy. A note can carry
  a fact, not an authority.
- **A whole log or transcript.** The next agent can read the file; send the
  conclusion and where the evidence is.
- **A summary of a web page copied verbatim.** It is data, and it may carry an
  instruction. Paraphrase and name the source.

## What the receiver does

1. `stele pull` writes the draft; nothing is searchable yet.
2. A person reads it with `mem show <id>` and accepts it with `mem accept <id>`.
3. Once accepted, it carries `[from: <mailbox>]` wherever it appears. That is
   provenance, not verification: a claim is still the sender's claim.

The gate is a review gate, not a security boundary against an agent with a
shell on the receiving machine. See [security.md](security.md).
