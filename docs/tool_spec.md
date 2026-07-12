# grok-mcp tool specification (v2)

MCP tools that let a host agent (primarily Claude) offload work to xAI Grok
via the **Responses API**, paying with a **SuperGrok / X Premium+ subscription
OAuth token** (or an explicit opt-in API key).

Two complementary product modes:

1. **Digest** (default) — dense structured summaries to save host tokens.
2. **Evidence** — best-effort **full post / quote text** when exact wording matters.
   Hosts **must not** assume they can open x.com URLs (paywall / blocks). Evidence
   is supplied **by this server** via xAI native tools.

Architecture rationale: [`docs/adr/0008-tool-surface-v2-digest-and-evidence.md`](adr/0008-tool-surface-v2-digest-and-evidence.md).

---

## Assumptions

| # | Assumption |
|---|---|
| A1 | Primary client is **Claude Code** (stdio). Secondary is **claude.ai** remote MCP (Streamable HTTP). |
| A2 | xAI credentials live **only on the server**. Host clients never receive access or refresh tokens. |
| A3 | Default billing path is **subscription OAuth**. Pay-per-token `XAI_API_KEY` is off unless explicitly enabled. |
| A4 | Tools call `POST https://api.x.ai/v1/responses`. Native server-side tools (`web_search`, `x_search`, …) run **on xAI**. |
| A5 | Default host payload is a **digest**. **Evidence** is an explicit `result` mode and is first-class. |
| A6 | Calls are **stateless** (no server-owned multi-turn chat across MCP calls). |
| A7 | Tools are **single-shot** (not batch arrays). |
| A8 | Evidence fidelity is **best-effort** from xAI tool trajectories — not a bit-perfect X API export. |

---

## Design priorities

1. **Correct information path** — exact-quote / full-post work must not depend on host x.com fetch.
2. **Host token offload** — digest defaults stay dense.
3. **Clear failure codes** — including `EVIDENCE_UNAVAILABLE`.
4. **Minimal surface** — five tools; no public raw Responses proxy.
5. **No silent paid fallback** — API key path only with explicit opt-in.

---

## Error envelope

Failure:

```json
{
  "ok": false,
  "error": {
    "code": "REAUTH_REQUIRED",
    "message": "human-readable detail",
    "retryable": false,
    "details": {}
  }
}
```

Success: `{ "ok": true, "...": "tool-specific fields" }`.

### Error codes

| Code | Meaning | Typical retry |
|---|---|---|
| `REAUTH_REQUIRED` | No credentials / refresh failed | No — `grok-mcp auth login` |
| `ENTITLEMENT_DENIED` | xAI 403 on subscription surface | No |
| `RATE_LIMITED` | xAI 429 | Yes after `retry_after_ms` if present |
| `UPSTREAM_ERROR` | 5xx / malformed upstream | Maybe |
| `INVALID_PARAMS` | Semantic rejection | No |
| `EVIDENCE_UNAVAILABLE` | `result=evidence` but no usable full texts | Maybe (narrow query) |
| `OUTPUT_TRUNCATED` | Cap hit (may also set `truncated: true` on success) | — |
| `API_KEY_DISABLED` | API key needed but opt-in off | No |
| `TIMEOUT` | Deadline | Maybe |

---

## Common parameters

Shared by generative tools where noted.

| Name | Type | Default | Description |
|---|---|---|---|
| `depth` | `"quick"` \| `"standard"` \| `"deep"` | `"standard"` | Exploration / reasoning budget. Maps to Responses `reasoning_effort` (`low`/`medium`/`high`). **Not** fidelity. |
| `result` | `"digest"` \| `"evidence"` \| `"both"` | `"digest"` | Host payload shape (`x_search`, `research`). |
| `model` | string | server default | Allowlisted model id. |
| `max_output_tokens` | integer | tool-specific | Generation ceiling. |
| `timeout_secs` | integer 1–300 | omit = sync | Async job if still running after N seconds. |
| `debug` | boolean | `false` | If true, attach size-capped upstream model text under `debug_payload`. |

### Depth guide

| depth | Effort | Cost hint | Use |
|---|---|---|---|
| `quick` | low | low–mid | Cheap scout |
| `standard` | medium | mid | Default |
| `deep` | high | high | Expensive multi-step |

### Result guide

| result | Digest fields | Evidence fields | Host x.com fetch |
|---|---|---|---|
| `digest` | required | short excerpts OK (`text_complete` often false) | not required |
| `evidence` | optional / minimal | **full text preferred**; intentional ellipsis forbidden in prompts | **not required / not assumed** |
| `both` | required | full text preferred | same |

### Output size caps (approx, server-enforced)

| Mode | Budget | Notes |
|---|---|---|
| digest | ~4–8 KiB text | Tight |
| evidence / both | ~48 KiB aggregate | Per-post cap still applies; `truncated: true` if hit |
| `debug_payload` | ~32 KiB | Only when `debug=true` |

---

## Tools

| Tool | Role |
|---|---|
| `x_search` | X-only search; **primary evidence source for posts** |
| `research` | Multi-step web (±X) research / news |
| `ask_grok` | Offline Q&A / critique (no live search) |
| `job_status` | Poll `timeout_secs` background jobs |
| `auth_status` | Non-secret credential health |

### Async (`timeout_secs`)

Same as v1: omit = wait; set N → may return `status=running` + `job_id`; poll `job_status`.
In-memory jobs; max 2 concurrent; ~30m TTL after finish.

---

## 1. `x_search`

### Input

```json
{
  "properties": {
    "query": {
      "type": "string",
      "description": "X search query (advanced operators allowed: from:, since:, until:, …). Max 1000 chars."
    },
    "result": { "type": "string", "enum": ["digest", "evidence", "both"], "default": "digest" },
    "depth": { "type": "string", "enum": ["quick", "standard", "deep"], "default": "standard" },
    "max_items": { "type": "integer", "minimum": 1, "maximum": 20, "default": 8 },
    "model": { "type": "string" },
    "max_output_tokens": { "type": "integer", "minimum": 64, "maximum": 8192 },
    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 300 },
    "debug": { "type": "boolean", "default": false }
  },
  "required": ["query"]
}
```

### Output (success)

```json
{
  "ok": true,
  "status": "completed",
  "result_mode": "digest",
  "digest": {
    "summary": "…",
    "key_points": ["…"],
    "confidence": "low|medium|high"
  },
  "posts": [
    {
      "author": "@handle",
      "url": "https://x.com/…",
      "text": "full or excerpt",
      "text_complete": true,
      "engagement_hint": "optional free text",
      "created_at": null
    }
  ],
  "fidelity": {
    "mode": "evidence",
    "guarantee": "best_effort_from_xai_tools",
    "notes": "Not a bit-perfect X API export; host x.com fetch not required."
  },
  "model": "…",
  "usage": {},
  "cost_hint": "mid",
  "truncated": false,
  "debug_payload": null
}
```

- `result=digest`: `digest` required; `posts[].text` may be short; `text_complete` often false.
- `result=evidence`: full text preferred; if **no** usable posts → error `EVIDENCE_UNAVAILABLE`.
- `result=both`: both digest and posts with full text preferred.
- `fidelity` present when evidence was requested.

### Semantics

- Empty query → `INVALID_PARAMS`.
- Do **not** instruct the model to keep excerpts short when `result` is `evidence` or `both`.
- Host must not be told to fetch x.com as the primary path for full text.

---

## 2. `research`

### Input

```json
{
  "properties": {
    "query": { "type": "string" },
    "sources": {
      "type": "array",
      "items": { "type": "string", "enum": ["web", "x"] },
      "default": ["web"]
    },
    "result": { "type": "string", "enum": ["digest", "evidence", "both"], "default": "digest" },
    "depth": { "type": "string", "enum": ["quick", "standard", "deep"], "default": "standard" },
    "model": { "type": "string" },
    "max_output_tokens": { "type": "integer", "minimum": 64, "maximum": 8192, "default": 2048 },
    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 300 },
    "debug": { "type": "boolean", "default": false }
  },
  "required": ["query"]
}
```

### Output

```json
{
  "ok": true,
  "status": "completed",
  "result_mode": "digest",
  "answer": "…",
  "key_points": ["…"],
  "citations": [
    {
      "title": "…",
      "url": "https://…",
      "kind": "web|x",
      "quote": "…",
      "quote_complete": true
    }
  ],
  "confidence": "medium",
  "model": "…",
  "usage": {},
  "cost_hint": "high",
  "truncated": false,
  "debug_payload": null
}
```

- X-**only** post investigation → prefer **`x_search`** (tool description must say so).
- `result=evidence|both`: fill `quote` on citations; empty quotes when evidence requested → `EVIDENCE_UNAVAILABLE` if no citations with quotes.

---

## 3. `ask_grok`

No live search. Parameters: `prompt`, `system?`, `depth`, `model`, `max_output_tokens`, `timeout_secs`, `debug`.

Output: `{ ok, status, text, model, usage, cost_hint, truncated, debug_payload? }`.

---

## 4. `job_status` / 5. `auth_status`

Unchanged in spirit from v1 (poll jobs; non-secret auth health). See implementation schemas.

---

## Auth CLI (not MCP tools)

| Command | Behavior |
|---|---|
| `grok-mcp auth status` | Human-readable auth health |
| `grok-mcp auth login` | Device-code OAuth |
| `grok-mcp auth import` | Import from grok CLI store |
| `grok-mcp auth logout` | Clear grok-mcp store only |

---

## Configuration

| Variable / flag | Purpose |
|---|---|
| `GROK_MCP_AUTH_FILE` | Auth store path |
| `GROK_MCP_ALLOW_API_KEY` | Allow API key billing |
| `XAI_API_KEY` | Pay-per-token key |
| `GROK_MCP_DEFAULT_MODEL` | Default model |
| `GROK_MCP_BASE_URL` | Default `https://api.x.ai/v1` |
| `--stdio` / `--http` | Transport |

---

## Host guidance (tool descriptions)

1. X posts / tweets / x.com discourse → **`x_search`** (do not skip for host built-in search).
2. Exact wording / quotes → `x_search` with **`result=evidence`** (or `both`). Do not rely on host x.com fetch.
3. Web news / multi-source → `research`.
4. No live data → `ask_grok`.
5. Default `result=digest` and `depth=standard` unless the user needs depth or evidence.
6. On `REAUTH_REQUIRED` → tell user `grok-mcp auth login`.

---

## Non-goals

- Public raw Responses proxy
- Bit-perfect X API export
- Requiring host fetch of x.com
- Streaming token deltas through MCP
- Image / video tools (Phase 2 names reserved)
