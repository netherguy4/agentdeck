#!/usr/bin/env bash
# Black-box suite for the agent-process port. Runs a given binary:
#   AGENT_PROCESS_BIN=/path/to/agent-process bash agent-process.test.sh
# Uses an isolated XDG_RUNTIME_DIR and only stops sleep processes it started.
set -euo pipefail

bin="${AGENT_PROCESS_BIN:?Set AGENT_PROCESS_BIN to the agent-process binary under test}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export XDG_RUNTIME_DIR="$tmp/runtime"
mkdir -p "$XDG_RUNTIME_DIR"
export AI_AGENT_SESSION_ID=suite-owner

started=()
fail() { printf 'FAILED: %s\n' "$*" >&2; exit 1; }
expect() { # expect <code> <command...>  (output captured to $tmp/out)
  local want="$1"; shift
  set +e
  "$@" >"$tmp/out" 2>"$tmp/err"
  local got=$?
  set -e
  [[ "$got" == "$want" ]] || { printf 'FAILED: %s -> exit %s, want %s\n' "$* $want" "$got" >&2; cat "$tmp/out" "$tmp/err" >&2; exit 1; }
}
stop_all() {
  for id in "${started[@]:-}"; do
    AI_AGENT_SESSION_ID=suite-owner "$bin" stop --force "$id" >/dev/null 2>&1 || true
  done
}
trap 'stop_all; rm -rf "$tmp"' EXIT

expect 0 "$bin"
grep -q '^Usage:' "$tmp/out" || fail "no-args usage"
expect 0 "$bin" --help
expect 2 "$bin" bogus
grep -q 'Unknown command: bogus' "$tmp/err" || fail "unknown command message"
expect 0 "$bin" list
grep -q '(no registered processes)' "$tmp/out" || fail "empty list"
expect 2 "$bin" start --name only
expect 2 "$bin" start --name x --cwd /nonexistent -- true
expect 2 "$bin" start --bogus -- true
expect 2 env -u AI_AGENT_SESSION_ID -u CODEX_THREAD_ID -u CODEX_SESSION_ID -u CLAUDE_SESSION_ID -u GEMINI_SESSION_ID -u OPENCODE_SESSION_ID \
  "$bin" start --name x -- /bin/true
grep -q 'Pass --owner' "$tmp/err" || fail "missing owner message"

expect 0 "$bin" start --name web --cwd /tmp -- /bin/sleep 60
id="$(grep '^  "id": ' "$tmp/out" | sed 's/.*: "//;s/".*//')"
[[ -n "$id" ]] || fail "start prints no id"
started+=("$id")
grep -q '"live": true' "$tmp/out" || fail "start live flag"
reg="$XDG_RUNTIME_DIR/ai-agent-processes/$id.json"
[[ -f "$reg" ]] || fail "registry entry missing"
for key in id name owner pid pgid start_ticks cwd command log started_at; do
  grep -q "\"$key\"" "$reg" || fail "registry key $key missing"
done
pid="$(grep '^  "pid": ' "$reg" | tr -cd '0-9')"
log="$(grep '^  "log": ' "$reg" | sed 's/.*: "//;s/".*//')"
[[ -f "$log" ]] || fail "log file missing"
tags="$(tr '\0' '\n' < "/proc/$pid/environ" | grep -c '^AI_AGENT_\(MANAGED=1\|PROCESS_ID='"$id"'\|PROCESS_OWNER=suite-owner\)$' || true)"
[[ "$tags" == 3 ]] || fail "child env tags (found $tags/3)"

expect 0 "$bin" list
grep -q "$id.*yes.*web.*suite-owner" "$tmp/out" || fail "list row"
expect 0 "$bin" status "$id"
grep -q '"live": true' "$tmp/out" || fail "status live"
# pid reuse: the registry identity (start_ticks + group tag) no longer matches
# the live process -> not live, and stop must not signal the real process.
cp "$reg" "$tmp/orig.json"
jq '.start_ticks = "0" | .pgid = 999999999' "$reg" >"$tmp/stale.json"
mv "$tmp/stale.json" "$reg"
expect 0 "$bin" status "$id"
grep -q '"live": false' "$tmp/out" || fail "reused pid looks live"
expect 0 "$bin" stop --owner suite-owner "$id"
grep -q 'no longer running' "$tmp/out" || fail "reused pid stop message"
kill -0 "$pid" 2>/dev/null || fail "stale identity stop killed the real process"
cp "$tmp/orig.json" "$reg"
expect 0 "$bin" status "$id"
grep -q '"live": true' "$tmp/out" || fail "restored identity not live"
expect 3 "$bin" stop --owner wrong-owner "$id"
grep -q 'Refusing:' "$tmp/err" || fail "owner refusal message"
expect 0 "$bin" status "$id"
grep -q '"live": true' "$tmp/out" || fail "refused stop keeps process live"
for bad in 0 301 01 nope 1.5; do
  expect 2 "$bin" stop --owner suite-owner --grace-seconds "$bad" "$id"
done
expect 2 "$bin" stop --bogus "$id"
expect 1 "$bin" status nosuchid
grep -q 'Unknown process ID: nosuchid' "$tmp/err" || fail "unknown status message"
expect 1 "$bin" stop --owner suite-owner nosuchid

expect 0 "$bin" stop --owner suite-owner "$id"
grep -q "Stopped $id" "$tmp/out" || fail "stop message"
expect 0 "$bin" status "$id"
grep -q '"live": false' "$tmp/out" || fail "status dead"
expect 0 "$bin" list
grep -q "$id.*no" "$tmp/out" || fail "list shows dead"
expect 0 "$bin" stop --owner suite-owner "$id"
grep -q 'no longer running' "$tmp/out" || fail "second stop message"

# --force lets another session stop a foreign process.
expect 0 "$bin" start --name foreign --owner other-owner -- /bin/sleep 60
fid="$(grep '^  "id": ' "$tmp/out" | sed 's/.*: "//;s/".*//')"
started+=("$fid")
expect 3 "$bin" stop "$fid"
expect 0 "$bin" stop --owner anyone --force "$fid"

# --no-escalate returns 4 and keeps a TERM-ignoring process registered.
expect 0 "$bin" start --name stubborn -- /bin/bash -c 'trap "" TERM; exec /bin/sleep 60'
sid="$(grep '^  "id": ' "$tmp/out" | sed 's/.*: "//;s/".*//')"
started+=("$sid")
expect 4 "$bin" stop --grace-seconds 1 --no-escalate "$sid"
grep -q 'no SIGKILL sent, registration retained' "$tmp/err" || fail "no-escalate message"
expect 0 "$bin" status "$sid"
grep -q '"live": true' "$tmp/out" || fail "no-escalate keeps process live"
expect 0 "$bin" stop "$sid"
grep -q 'SIGKILL' "$tmp/out" || fail "escalation message"

printf '%s\n' 'agent-process CLI tests passed.'
