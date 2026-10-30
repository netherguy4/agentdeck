# Migrating ai-usage to agentdeck

Owner-run cutover. Agents automate checks only: they never stop, disable,
enable or start services, and never delete the old install. Lines marked
OWNER need you at the terminal at that moment.

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

## 3. OWNER cutover: switch polling

```sh
systemctl --user stop ai-usage.service
systemctl --user enable --now agentdeck-daemon.service
agentdeck usage doctor   # new updated_at fresh for all sources
```

Dual-write keeps the router and old applet builds working: the new daemon
still writes the old state paths.

## 4. Rollback (before step 5)

```sh
systemctl --user stop agentdeck-daemon.service
systemctl --user start ai-usage.service   # old tree was copied, never moved
```

## 5. OWNER retirement (soak >= 1 week, then all by hand)

```sh
systemctl --user disable ai-usage.service
rm ~/.local/bin/ai-usage*
# remove the GNOME extension dir, then relogin (Wayland cannot unload it live)
rm -rf ~/.config/ai-usage ~/.local/state/ai-usage
```

## 6. Hooks

```sh
agentdeck usage hooks plan    # review what would change
# apply per the plan output (writes only with an explicit flag, never by plan)
```
