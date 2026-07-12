# 8. Tool surface v2: digest and evidence

## Status

Accepted

## Context

ADR-0004 optimized only for **dense host-token offload**. Real use showed:

1. `verbosity=raw` did not deliver full X post text — hosts need **exact wording**.
2. The fallback “URL + host fetch” fails: x.com blocks scrapers; host fetch is often paid/unavailable.
3. One axis (`verbosity`) mixed density, debug dumps, and (falsely) fidelity.

Development-stage contracts may break; a clean split is cheaper than patching `raw`.

## Decision

We will **supersede ADR-0004** for the public MCP tool contract (see
[`docs/tool_spec.md`](../tool_spec.md) v2):

1. **Dual product modes** on live tools (`x_search`, `research`):
   - `result=digest` (default) — summaries for scout/sentiment.
   - `result=evidence` — best-effort full post/quote text; **no host x.com fetch required**.
   - `result=both` — digest plus evidence posts/citations.
2. **`depth`** (`quick` \| `standard` \| `deep`) replaces `reasoning_effort` and drives
   exploration budget; not the same as result fidelity.
3. **Remove `verbosity` and `raw`** from the public contract. Optional `debug=true`
   may attach a size-capped upstream payload for operators only.
4. **Evidence is best-effort** from xAI native tool trajectories, not a bit-perfect
   X API export. Empty evidence when `result=evidence` → `EVIDENCE_UNAVAILABLE`.
5. Keep the small tool set: `research`, `x_search`, `ask_grok`, `job_status`,
   `auth_status`. No public raw Responses proxy.
6. Breaking change → **semver minor `0.2.0`** (pre-1.0 API break).

## Consequences

- Positive: hosts can quote X posts without paid host fetch of x.com.
- Positive: digest path remains cheap for discourse scouting.
- Positive: parameters match concepts hosts already reason about (depth vs evidence).
- Negative: Grok may still paraphrase; `text_complete` and error codes mitigate silence.
- Negative: evidence mode costs more SuperGrok quota and larger host payloads.
- Neutral: ADR-0004 dense-offload goal remains for `result=digest` defaults.
