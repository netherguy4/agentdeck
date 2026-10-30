// Provider instances sharing one subscription or credit balance compete for the same quota.
// codex_second is a separate Codex Plus login (shadow CODEX_HOME); claude_second/claude_third/claude_fourth are separate Claude logins.
export const ACCOUNT = { claudeAgent: 'claude', claude_second: 'claude_second', claude_third: 'claude_third', claude_fourth: 'claude_fourth', codex: 'codex',
  codex_second: 'codex_second', antigravity: 'antigravity', opencode: 'openrouter', claudeAgent_openrouter: 'openrouter' };
// ai-usage reports each Codex and Claude login under one provider name; only their account ids tell them apart.
const USAGE_ID = { codex: 'codex-main', codex_second: 'codex-second',
  claudeAgent: 'claude-main', claude_second: 'claude-second', claude_third: 'claude-third', claude_fourth: 'claude-fourth' };
// The OpenCode instance mixes OpenRouter credits with the OpenCode Go subscription; the model prefix decides.
export const accountOf = (instance, model) =>
  instance === 'opencode' && /^opencode-go\//.test(model ?? '') ? 'opencode-go' : ACCOUNT[instance];
export function ownsUsage(account, instance, model) {
  return USAGE_ID[instance] && account.id ? account.id === USAGE_ID[instance] : account.provider === accountOf(instance, model);
}
export const MIN_FIT = 0.7;
// A requested delegate may differ from the selected best only by this many score points.
export const TARGET_MARGIN = 0.02;

export function enabledFavorites(favorites, settings) {
  return favorites.filter(f => (settings.providerInstances?.[f.provider]?.enabled
    ?? settings.providers?.[f.provider]?.enabled) !== false);
}

export function quotaWindows(provider, model, windows) {
  return windows.filter(w => {
    // ai-usage marks the currently active Antigravity family unscoped, not every candidate's family.
    if (provider === 'antigravity') {
      // Argon draws only on its own weekly bucket: a measured task moved it 7.2% and left gemini-5h/weekly at 0 (2026-10-16).
      if (/^gemini[-_].*argon/i.test(w.key)) return /argon/i.test(model);
      if (/^gemini[-_]/i.test(w.key)) return /gemini/i.test(model) && !/argon/i.test(model);
      if (/^(3p|third[-_]party)[-_]/i.test(w.key)) return !/gemini/i.test(model);
    }
    return !w.scope || model.toLowerCase().includes(w.scope.toLowerCase());
  });
}

// ponytail: dollar costs and the default 5-hour factor are guesses; Codex windows carry a measured costFactor.
// Replace the rest with measurements once ai-usage keeps history.
const windowFactor = w => w.duration_mins >= 10080 ? 1 : w.costFactor ?? 5;
const USD_COST = [0.05, 0.3, 1, 3];
export function reserveActiveQuota(account, agents, costsOf) {
  const active = agents.filter(a => ownsUsage(account, a.provider, a.model));
  // ponytail: reserve one focused task per active agent; use actual remaining task size when T3 exposes it.
  const costs = active.map(a => ({ ...a, ...costsOf(a.provider, a.model) }));
  const windows = account.windows.map(w => {
    const pending = costs.filter(a => !a.model || quotaWindows(a.provider, a.model, [w]).length)
      .reduce((total, a) => total + Math.max(0.1, a.weekly[1] * windowFactor(w)), 0);
    return { ...w, pending, remaining_percent: Math.max(0, w.remaining_percent - pending) };
  });
  const pendingDollars = costs.reduce((total, a) => total + USD_COST[1] * a.burn, 0);
  return { activeAgents: active.length, windows,
    balances: account.balances.map(b => Math.max(0, Number(b.total) - pendingDollars)), pendingDollars };
}

// allowed(size): the task fits every applicable window and balance. Low remaining quota is not penalized
// otherwise (user rule 2026-10-17). cost(size): estimated task cost as a fraction of spendable budget,
// used only to break exact ties and near-equal requested targets.
export function quotaBudget(windows, balances, weekly, burn, reservePercent, now = Date.now() / 1000) {
  const timeLeft = w => Number.isFinite(w.resets_at) && w.resets_at > now && w.duration_mins > 0
    ? Math.min(1, (w.resets_at - now) / (w.duration_mins * 60)) : null;
  const headroom = w => Math.max(0, w.remaining_percent - (w.duration_mins >= 10080
    ? Math.min(reservePercent, (timeLeft(w) ?? 1) * 100) : 0));
  const windowCost = (w, size) => weekly[size] * windowFactor(w);
  const known = windows.length || balances.length;
  // Measured zero means below the log's percent resolution, not free work.
  const cost = size => known ? Math.max(
    ...windows.map(w => Math.max(0.1, windowCost(w, size)) / Math.max(0.001, headroom(w))),
    ...balances.map(b => USD_COST[size] * burn / Math.max(0.001, b))) : null;
  return {
    allowed: size => windows.every(w => headroom(w) > windowCost(w, size))
      && balances.every(b => b > USD_COST[size] * burn),
    cost,
  };
}

// Audits and reviews of any kind (code, design, UI/UX). JS \b does not cover Cyrillic, hence the bare stems.
export const REVIEW_TASK = /\baudit|\breview|аудит|ревью|рев['’ʼ]ю/i;
export const NO_REVIEW_BAN = 'never for audits or reviews (user rule 2026-10-17)';

// Score is one scalar per candidate, so the ranking stays transitive. Remaining quota only gates eligibility.
export function rankCandidates(candidates, difficulty, source, weights) {
  const account = ACCOUNT[source];
  const scale = difficulty > 1 ? 0.5 : 1;
  // Jev judges one model separately per account, so the copies differ only by sampling noise (0.86 vs 0.84 for
  // the same gpt-6.1-sol, 2026-10-17). Their mean gives every copy one verdict; the load tie-break then spreads work.
  const copies = Map.groupBy(candidates, c => c.model);
  const mean = (c, k) => {
    const same = copies.get(c.model);
    return same.length > 1 && same.every(s => Number.isFinite(s[k])) ? same.reduce((t, s) => t + s[k], 0) / same.length : c[k];
  };
  return candidates.map(c => ({ ...c, quality: mean(c, 'quality'), fit: mean(c, 'fit'), confidence: mean(c, 'confidence') })).map(c => {
    const own = accountOf(c.provider, c.model);
    const sharedQuota = Boolean(account && own === account);
    const sharedQuotaPenalty = sharedQuota ? weights.sharedQuotaPenalty * scale : 0;
    const paidCreditPenalty = own === 'openrouter' ? weights.paidCreditPenalty * scale : 0;
    // Rewards models that are cheap per task (burn < 1), never remaining quota; unknown burn earns nothing.
    const cheapModelBonus = weights.cheapModelBonus * Math.max(0, 1 - (c.burn ?? 1)) * scale;
    return { ...c, account: own ?? null, sharedQuota, sharedQuotaPenalty, paidCreditPenalty, cheapModelBonus, eligible: Boolean(c.allowed && c.profiled !== false && !c.reasoningError && !c.policyBan && c.fit >= MIN_FIT),
      score: c.quality - sharedQuotaPenalty - paidCreditPenalty + cheapModelBonus,
    };
  }).sort((a, b) => Number(b.eligible) - Number(a.eligible) || b.score - a.score
    || (a.activeAgents ?? 0) - (b.activeAgents ?? 0) || (a.taskCost ?? 1) - (b.taskCost ?? 1) || b.fit - a.fit);
}

// Jev score answers: score in 0..3, confidence and every level probability in 0..1, probabilities summing to 1.
const unit = x => Number.isFinite(x) && x >= 0 && x <= 1;
export function validAnswer(a, requireProbabilities = true) {
  if (!a || !Number.isFinite(a.score) || a.score < 0 || a.score > 3 || !unit(a.confidence)) return false;
  if (!requireProbabilities && a.probabilities === undefined) return true;
  const p = ['0', '1', '2', '3'].map(k => a.probabilities?.[k]);
  return p.every(unit) && Math.abs(p.reduce((s, x) => s + x, 0) - 1) <= 0.02;
}

// Guards a delegate call the orchestrator already chose: it must be routable now and nearly as good as the best.
export function validateTarget(ranking, requested) {
  const r = ranking.find(c => c.provider === requested.providerInstanceId && c.model === requested.model);
  const best = ranking.find(c => c.eligible);
  const deny = reason => ({ allowed: false, reason });
  if (r?.policyBan) return { allowed: false, reason: r.policyBan };
  if (!r) return deny('not an exact active favorite');
  if (r.profiled === false) return deny('favorite has no capability profile');
  if (r.reasoningError) return deny(r.reasoningError);
  if (!r.allowed) return deny('not enough current quota for this task size');
  if (!(r.fit >= MIN_FIT)) return deny(`fit ${r.fit.toFixed(3)} below ${MIN_FIT}`);
  if (r === best) return { allowed: true, reason: 'selected winner' };
  const gap = best.score - r.score;
  if (gap <= TARGET_MARGIN && (r.taskCost ?? 1) > (best.taskCost ?? 1) + 1e-9)
    return deny('near-equal requested model costs more than the selected winner');
  return gap <= TARGET_MARGIN
    ? { allowed: true, reason: `within ${TARGET_MARGIN} of ${best.provider}/${best.model} (gap ${gap.toFixed(3)})` }
    : deny(`score ${gap.toFixed(3)} below ${best.provider}/${best.model}; margin is ${TARGET_MARGIN}`);
}
