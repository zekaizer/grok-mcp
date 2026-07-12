# 4. Tool surface for host token offload

## Status

Superseded by [ADR-0008](0008-tool-surface-v2-digest-and-evidence.md)

## Context

Host agents (Claude) pay for every token they generate **and** every tool result
they ingest. Naively wrapping xAI search so that large raw payloads return to
Claude can *increase* host cost.

The user's goals:

1. Use under-utilized SuperGrok subscription quota.
2. Reduce Claude subscription token spend.
3. Keep Claude Code as the main agent; expose xAI-native capabilities (especially
   X search and multi-step research) when useful.

Three product shapes were considered:

- **Thin relay** — one raw `respond` tool. Minimal code; poor host guidance;
  easy to return huge blobs.
- **Capability passthrough** — mirror each native tool (`web_search`, `x_search`,
  …) with raw-ish outputs. Clear, but weak offload economics.
- **Dense offload tools** — fewer tools that do more work on Grok and return
  structured digests. Best match for token savings; slightly more server logic.

## Decision

We will expose a **small Phase-1 MCP tool surface** defined in
[`docs/tool_spec.md`](../tool_spec.md):

| Tool | Purpose |
|---|---|
| `research` | Multi-step research with optional web/X native tools → structured digest |
| `x_search` | X-focused search → summary + short post list |
| `ask_grok` | Single-shot sub-LLM offload (critique, analysis, Q&A) |
| `auth_status` | Non-secret credential health |

Normative rules:

1. **Dense default:** `verbosity` defaults to `summary`; `raw` is opt-in and
   size-capped.
2. **No public raw Responses proxy** in Phase 1.
3. **Stateless calls** — no server-owned multi-turn sessions across MCP invocations.
4. **Shared error envelope** with machine-readable `code` values for reauth,
   entitlement, rate limit, and validation failures.
5. **Tool descriptions** must steer the host to use these tools for expensive
   multi-turn host work and to avoid redundant re-search when confidence is adequate.

Phase 2 tools (`web_search`, `code_execution`, multi-agent, media) require a
new ADR or an additive revision of `tool_spec.md` plus an ADR if the contract
philosophy changes.

## Consequences

- Positive: aligns the API with the actual economic goal (Claude tokens down,
  SuperGrok utilization up).
- Positive: four tools are easy for hosts to learn and for us to test.
- Negative: hosts that want raw post JSON must use `verbosity=raw` within caps
  or wait for Phase 2.
- Negative: summary quality depends on Grok; bad digests may cause host re-work
  (partially mitigated by `confidence` and `sources`).
- Neutral: implementation maps tools onto Responses + native tool flags inside
  `grok-client`, not onto separate public HTTP routes.
