# agent-process (Rust port)

Identical CLI, exit codes, registry layout and child env tags as the
`agent-process` bash helper. Spawning/signalling reuses the same `setsid`,
`env`, `kill` and `id` binaries, so the process tree and observable behaviour
match; only new dependency is `serde_json` (with `preserve_order`, so registry
key order matches `jq`).

Behaviour that could not be matched 1:1:

- Missing ID/value errors lack bash's locale-dependent `file: line N:` prefix;
  the message text and exit codes (1) are preserved.
- Unparseable `*.json` registry entries are skipped by `list`; the script
  prints a blank-padded row for them (both exit 0).
- `start_ticks` reads `/proc/<pid>/stat` field 22 exactly like the script's
  `awk '{print $22}'`, including its behaviour on exotic command names.
