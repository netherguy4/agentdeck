import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Codex session-log calibration through the actual route.mjs --check, with an isolated home.
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'model-route-costs-test-'));
try {
  const sessions = path.join(dir, '.codex/sessions/2026/10/01');
  for (const rel of ['.t3/userdata', '.local/state/ai-usage']) fs.mkdirSync(path.join(dir, rel), { recursive: true });
  fs.mkdirSync(sessions, { recursive: true });
  fs.writeFileSync(path.join(dir, '.t3/userdata/client-settings.json'), JSON.stringify({
    favorites: ['gpt-x', 'gpt-y', 'gpt-z', 'gpt-w', 'gpt-v'].map(model => ({ provider: 'codex', model })) }));
  fs.writeFileSync(path.join(dir, '.t3/userdata/settings.json'), '{}');
  // Legacy caches (v4 includes an earlier draft's calibration) and an unversioned current cache must not be trusted.
  const bogus = JSON.stringify({ plan: 'plus', costs: { 'prolite|gpt-x': { n: 99, pct: [9, 9, 9, 9] } } });
  fs.writeFileSync(path.join(dir, 'model-route-codex-costs-v2.json'), bogus);
  fs.writeFileSync(path.join(dir, 'model-route-codex-costs-v4.json'), JSON.stringify({ version: 4,
    costs: { 'prolite|gpt-x': { n: 99, pct: [9, 9, 9, 9] } } }));
  fs.writeFileSync(path.join(dir, 'model-route-codex-costs-v7.json'), bogus);

  const start = Date.now() - 3 * 86400e3;
  const R = Math.round(Date.now() / 1000) + 3 * 86400;
  const week = (used, resets) => ({ used_percent: used, window_minutes: 10080, resets_at: resets });
  const logs = {}, totals = {};
  // repeat: Codex re-logs the previous cumulative total with a non-zero last_token_usage; no new tokens.
  const event = (file, minute, model, plan, rl, tokens = 1000, repeat = false) => (logs[file] ??= []).push(
    JSON.stringify({ timestamp: new Date(start + minute * 60e3).toISOString(), type: 'turn_context', payload: { model } }),
    JSON.stringify({ timestamp: new Date(start + minute * 60e3).toISOString(), type: 'event_msg', payload: { type: 'token_count',
      info: { last_token_usage: { total_tokens: tokens },
        total_token_usage: { total_tokens: totals[file] = (totals[file] ?? 0) + (repeat ? 0 : tokens) } },
      rate_limits: { ...rl, plan_type: plan } } }));
  const pro = (used, resets = R) => ({ primary: week(used, resets), secondary: null });
  // Pro 5x gpt-x: 1 point per 1000 tokens, with reset jitter of a few seconds; 3000-token sessions cost 3%.
  for (let i = 0; i < 10; i++) {
    for (let k = 0; k < 3; k++) event(`x${i}`, i * 10 + k, 'gpt-x', 'prolite', pro(10 + i * 3 + k, R + [0, 5, -11][k]));
    event(`x${i}`, i * 10 + 2.5, 'gpt-x', 'prolite', pro(12 + i * 3), 1000, true);
  }
  // A stale snapshot jump inside a session is not consumption.
  event('x0', 3, 'gpt-x', 'prolite', pro(60), 0);
  event('x0', 4, 'gpt-x', 'prolite', pro(12), 0);
  // A session mixing both accounts cannot be one account's sample.
  event('mix', 150, 'gpt-x', 'prolite', pro(45));
  event('mix', 151, 'gpt-x', 'plus', { primary: { used_percent: 1, window_minutes: 300, resets_at: R }, secondary: week(1, R + 3600) });
  // Pro 5x gpt-y: two sessions interleave, so no interval is attributable to either.
  for (let i = 0; i < 10; i += 2) for (let k = 0; k < 3; k++) for (const j of [i, i + 1])
    event(`y${j}`, 200 + i * 10 + k * 2 + (j - i), 'gpt-y', 'prolite', pro(50 + i * 3 + k * 2 + (j - i)));
  // Pro 5x gpt-z: each session spans a weekly reset; the 90 -> 1 drop is not a delta.
  for (let i = 0; i < 10; i++) {
    event(`z${i}`, 400 + i * 10, 'gpt-z', 'prolite', pro(90, R - 7 * 86400));
    event(`z${i}`, 401 + i * 10, 'gpt-z', 'prolite', pro(1 + i, R));
    event(`z${i}`, 402 + i * 10, 'gpt-z', 'prolite', pro(2 + i, R));
  }
  // Pro 5x gpt-w: another session's delayed snapshot lands just before w's own interval, leaking +1 point
  // into the k=2 -> k=3 delta. Only k=0 -> k=1 is clean: 1 point per 1000 tokens, 4000-token sessions cost 4%.
  for (let i = 0; i < 10; i++) {
    const m = 800 + i * 10, u = 70 + i * 5;
    event(`w${i}`, m, 'gpt-w', 'prolite', pro(u));
    event(`w${i}`, m + 1, 'gpt-w', 'prolite', pro(u + 1));
    event(`f${i}`, m + 1.5, 'gpt-q', 'prolite', pro(u + 1), 0);
    event(`w${i}`, m + 2, 'gpt-w', 'prolite', pro(u + 2));
    event(`w${i}`, m + 3, 'gpt-w', 'prolite', pro(u + 4));
  }
  // Pro 5x gpt-v, its own clean stream: each helper session h's consumption shows up only in the next v
  // interval, which looks quiet (h logged 3 minutes earlier), so quiet intervals alone would say 2 points per
  // 1000 tokens (4% per session). The whole stream is 29 points over 38000 attributable tokens: 1.53%.
  const R2 = R + 2 * 86400;
  for (let i = 0; i < 10; i++) {
    const m = 1000 + i * 10, u = 5 + i * 3;
    event(`h${i}`, m, 'gpt-v', 'prolite', pro(u, R2), 2000);
    event(`v${i}`, m + 3, 'gpt-v', 'prolite', pro(u, R2));
    event(`v${i}`, m + 4, 'gpt-v', 'prolite', pro(u + 2, R2));
  }
  // The newest events are another account on Plus: 5-hour primary +2, weekly secondary +1 per 1000 tokens.
  for (let i = 0; i < 10; i++) for (let k = 0; k < 2; k++) event(`p${i}`, 600 + i * 10 + k, 'gpt-x', 'plus', {
    primary: { used_percent: 10 + k * 2, window_minutes: 300, resets_at: R - 86400 },
    secondary: week(20 + i * 2 + k, R + 3600) });
  for (const [name, lines] of Object.entries(logs)) fs.writeFileSync(path.join(sessions, `${name}.jsonl`), lines.join('\n'));
  fs.writeFileSync(path.join(sessions, 'oversized.jsonl'), '');

  const check = plan => {
    fs.writeFileSync(path.join(dir, '.local/state/ai-usage/state.json'), JSON.stringify({ accounts: [{
      provider: 'codex', plan, status: 'ok', updated_at: Date.now() / 1000, balances: [],
      windows: [{ key: 'primary', label: 'Week', remaining_percent: 50, duration_mins: 10080, resets_at: R }] }] }));
    const out = execFileSync(process.execPath, ['--input-type=module', '-e', `
      import os from 'node:os';
      import fs from 'node:fs';
      const read = fs.readFileSync;
      fs.readFileSync = (file, ...args) => {
        if (String(file).endsWith('/oversized.jsonl')) throw new Error('ERR_STRING_TOO_LONG');
        return read(file, ...args);
      };
      os.homedir = () => process.cwd();
      process.argv = [process.execPath, 'route.mjs', '--check'];
      await import(${JSON.stringify(new URL('./route.mjs', import.meta.url).href)});
    `], { cwd: dir, env: { ...process.env, XDG_RUNTIME_DIR: dir }, encoding: 'utf8' });
    return model => out.split(`codex/${model}\n`)[1].split('allowed sizes')[0];
  };
  const pro5x = check('Pro 5x');
  assert.match(pro5x('gpt-x'), /3 \/ 3 \/ 3 \/ 3 \(measured over 10 sessions on Pro 5x\)/);
  assert.match(pro5x('gpt-y'), /Pro 5x estimate/);
  assert.match(pro5x('gpt-z'), /3 \/ 3 \/ 3 \/ 3 \(measured over 10 sessions on Pro 5x\)/);
  assert.match(pro5x('gpt-w'), /4 \/ 4 \/ 4 \/ 4 \(measured over 10 sessions on Pro 5x\)/);
  assert.match(pro5x('gpt-v'), /1\.53 \/ 1\.53 \/ 1\.53 \/ 1\.53 \(measured over 20 sessions on Pro 5x\)/);
  // Weekly secondary, not the 5-hour primary percentage.
  assert.match(check('Plus')('gpt-x'), /2 \/ 2 \/ 2 \/ 2 \(measured over 10 sessions on Plus\)/);
  assert.match(check('Pro 20x')('gpt-x'), /Pro 20x estimate/);
  const cache = JSON.parse(fs.readFileSync(path.join(dir, 'model-route-codex-costs-v7.json'), 'utf8'));
  assert.equal(cache.version, 7);
  // Plus fixture: 5-hour +2 per weekly +1; Pro 5x logs no 5-hour window.
  assert.deepEqual(cache.shortFactor, { plus: 2 });
} finally { fs.rmSync(dir, { recursive: true }); }
console.log('codex cost calibration checks passed');
