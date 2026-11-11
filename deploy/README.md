# Deploying stele

Systemd user units, no sudo. The mailbox runs on the machine that is always
on; `pull` runs wherever the messages should land.

## Build and install

```bash
cargo build --release
install -m 755 target/release/stele ~/.local/bin/stele
```

## The mailbox

```bash
# one token per machine; each is printed once, on stdout
stele --db ~/.local/share/stele/stele.db token add dell
stele --db ~/.local/share/stele/stele.db token add laptop

# edit the bind address in the unit first — it has to be the Tailscale IP
mkdir -p ~/.config/systemd/user
cp deploy/stele-serve.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now stele-serve
systemctl --user status stele-serve
```

The unit prints `listening on http://<address>` on start. Check it from
another machine on the tailnet:

```bash
curl -s http://<tailscale-ip>:3900/health   # {"ok":true}
```

If the service stops when you close the session, the user manager needs to
linger: `loginctl enable-linger $USER` (this may need sudo once).

## Every client machine

```bash
stele login --url http://<tailscale-ip>:3900 --token -
# paste the token printed by `token add`, then Ctrl-D
stele whoami     # must print the mailbox name
```

## Receiving

```bash
cp deploy/stele-pull.service deploy/stele-pull.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now stele-pull.timer
systemctl --user list-timers stele-pull.timer
```

The drafts are then in anathem:

```bash
mem recent --all --external
mem show 42
mem accept 42
```

`--all` matters: `mem recent` defaults to the project derived from the
directory you are in, and the note may be filed under another one.

`mem` has to be on the service's `PATH`; the unit above adds `~/.local/bin`,
which is where the installer puts it. If yours lives somewhere else, set
`mem_bin` in `~/.config/stele/config.json`.

## Moving or rotating

```bash
stele --db ~/.local/share/stele/stele.db token rotate laptop
# on the laptop, log in again with the new token
stele --db ~/.local/share/stele/stele.db token list
stele --db ~/.local/share/stele/stele.db token remove old-laptop
```

Removing a mailbox also deletes its undelivered mail.

## Backups

The whole state is `~/.local/share/stele/stele.db` (plus the WAL while the
service runs). Copy it with the service stopped, or use `sqlite3 stele.db
".backup backup.db"`. Messages expire after seven days anyway; the archive
that matters is anathem, on the receiving side.
