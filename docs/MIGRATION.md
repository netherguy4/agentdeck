# Migrating ai-usage to agentdeck

Owner-run cutover. Agents automate checks only: they never stop, disable,
enable or start services, and never delete the old install. Lines marked
OWNER need you at the terminal at that moment.

## 0. Install the binary and fill the registry

```sh
cargo install --locked --path crates/agentdeck   # hooks and the unit point at this path
agentdeck t3 import --dry-run
agentdeck t3 import     # instances first: migrate links usage sources to them by login dir
```

## 1. Copy the old config (no daemon running yet)

```sh
agentdeck migrate ai-usage --dry-run   # inspect the id → instance/usage table
agentdeck migrate ai-usage              # copy-only: ids, secrets, cooldowns
agentdeck usage doctor                  # 9 sources, backoff ages, state paths
agentdeck usage once --dry-run          # single manual poll, writes nothing
```

## 2. Install the new unit (no polling yet)

```sh
agentdeck service install --dry-run
agentdeck service install   # renders the unit, daemon-reload only
agentdeck service status    # installed: yes, active: no
```

Smoke-test dual-write against the still-running old daemon:

```sh
agentdeck usage once --compat-write-ai-usage
diff <(jq -S . ~/.local/state/agentdeck/usage/state.json) \
     <(jq -S . ~/.local/state/ai-usage/state.json)  # expect only updated_at skew
```

## 3. Re-point hooks (before cutover)

Hooks of the old binary write into the old state dir, which the new daemon no
longer reads for snapshots.

```sh
agentdeck usage hooks plan   # shows each hook's location, current and desired command
```

OWNER: replace the `current` command with the `desired` one at each `location`.

## 4. OWNER cutover: switch polling

```sh
systemctl --user stop ai-usage.service
systemctl --user enable --now agentdeck-daemon.service
agentdeck usage doctor   # new updated_at fresh for all sources
```

Dual-write keeps the router and old applet builds working: the new daemon
still writes the old state paths.

## 5. Rollback (before step 6)

```sh
systemctl --user stop agentdeck-daemon.service
systemctl --user start ai-usage.service   # old tree was copied, never moved
# put the old hook commands back if step 3 was applied
```

## 6. OWNER retirement (soak >= 1 week, then all by hand)

```sh
systemctl --user disable ai-usage.service
rm ~/.local/bin/ai-usage*
# remove the GNOME extension dir, then relogin (Wayland cannot unload it live)
rm ~/.config/ai-usage/config.toml ~/.config/ai-usage/secrets.env
```

Keep `~/.config/ai-usage/hooks/` (the agy hook script lives there) and
`~/.local/state/ai-usage/` (the router reads `state.json` from it until it moves
to the pool in M6; the daemon keeps writing it).

