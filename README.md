# stele

A *stele* is a stone slab with an inscription, set upright where whoever comes
next will find it. This one carries a message from one coding agent to another
and leaves it in [anathem](https://gitlab.com/__alexander__/anathem) as a draft.

**Status: design only.** There is no code yet; `docs/design.md` is the proposal.

```
stele send --to opencode "The API contract changed to v2"
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

Design, transport, dependencies and the resource targets:
[`docs/design.md`](docs/design.md).

## License

MIT. See [`LICENSE`](LICENSE).
