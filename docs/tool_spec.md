# grok-mcp tool specification

MCP tools that let a host agent (primarily Claude) offload work to xAI Grok
via the **Responses API**, paying with a **SuperGrok / X Premium+ subscription
OAuth token** (or an explicit opt-in API key). Outputs are **dense by default**
so the host spends fewer of its own tokens.

This document is the public tool contract. Architecture rationale lives in
[`docs/adr/`](adr/).

---

## Assumptions

| # | Assumption |
|---|---|
| A1 | Primary client is **Claude Code** (stdio). Secondary is **claude.ai** remote MCP (Streamable HTTP). |
| A2 | xAI credentials live **only on the server**. Host clients never receive access or refresh tokens. |
| A3 | Default billing path is **subscription OAuth**. Pay-per-token `XAI_API_KEY` is off unless explicitly enabled. |
| A4 | Tools call `POST https://api.x.ai/v1/responses` (or documented media endpoints later). Native server-side tools (`web_search`, `x_search`, `code_interpreter`) run **on xAI**, not in this process. |
| A5 | Results returned to the host are **summaries / structured digests** unless the caller opts into raw payloads. |
| A6 | Phase 1 is **stateless** per tool call (no server-owned multi-turn chat sessions). Each call is self-contained. |
| A7 | All tools are **single-shot** (not batch arrays). Batching is deferred until real host patterns demand it. |

---

## Design priorities

1. **Host token offload** — maximize work done on Grok per byte returned to Claude.
2. **Dense default** — `verbosity` defaults to `summary`; raw is opt-in and size-capped.
3. **Clear failure codes** — agents can recover without re-probing (`REAUTH_REQUIRED`, `RATE_LIMITED`, …).
4. **Minimal surface** — four Phase-1 tools; no raw `respond` escape hatch in the public set.
5. **No silent paid fallback** — API key path never engages without explicit config.

---

## Error envelope

Every tool returns JSON. On failure:

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

On success:

```json
{
  "ok": true,
  "...": "tool-specific fields"
}
```

### Error codes (shared)

| Code | Meaning | Typical retry |
|---|---|---|
| `REAUTH_REQUIRED` | Refresh failed / no credentials / grant revoked | No — run `grok-mcp auth login` |
| `ENTITLEMENT_DENIED` | xAI 403 on subscription OAuth surface | No — check subscription or enable API key opt-in |
| `RATE_LIMITED` | xAI 429 | Yes after `details.retry_after_ms` if present |
| `UPSTREAM_ERROR` | 5xx or malformed upstream body | Maybe |
| `INVALID_PARAMS` | Schema-pass but semantic rejection (empty query, caps) | No — fix args |
| `OUTPUT_TRUNCATED` | Success path with hard size cap hit (may still set `ok: true` with a flag) | — |
| `API_KEY_DISABLED` | Request would need API key but opt-in is off | No |
| `TIMEOUT` | Upstream or local deadline | Maybe |

Schema violations (`required`, types, enums) are rejected by the MCP framework before the handler; they are not wrapped in this envelope.

---

## Common parameters

Shared by `research`, `x_search`, and `ask_grok` where noted.

| Name | Type | Default | Description |
|---|---|---|---|
| `verbosity` | `"summary"` \| `"detailed"` \| `"raw"` | `"summary"` | Output density. `summary` = short structured digest. `detailed` = longer analysis + more sources. `raw` = include upstream text/tool payloads up to size caps (debug / rare). |
| `model` | string | server default (config) | xAI model id. Server validates against an allowlist. |
| `reasoning_effort` | `"low"` \| `"medium"` \| `"high"` | `"medium"` for `research`/`ask_grok`; ignored or forced low for pure search when applicable | Maps to Responses `reasoning_effort` when the model supports it. |
| `max_output_tokens` | integer | tool-specific | Hard cap on model generation; server also enforces a global ceiling. |

### Output size caps (server-enforced)

| verbosity | Max returned text (approx) | Notes |
|---|---|---|
| `summary` | 4 KiB | Prefer bullets + source list |
| `detailed` | 16 KiB | |
| `raw` | 32 KiB | Truncate with `truncated: true` |

Exact byte limits are implementation constants; tools report `truncated: true` when applied.

---

## Phase 1 tools

| Tool | Role |
|---|---|
| `research` | Multi-step research on a topic using Grok + optional web/X tools; returns a structured digest |
| `x_search` | Search X (Twitter) via xAI native `x_search`; dense results |
| `ask_grok` | Single-shot question, critique, or analysis (sub-LLM offload, no search) |
| `job_status` | Poll a background job started when `timeout_secs` returned `status=running` |
| `auth_status` | Credential health without exposing secrets |

### Async wait (`timeout_secs`)

`research`, `ask_grok`, and `x_search` accept optional `timeout_secs` (1–300).

- **Omitted:** fully synchronous — wait until the xAI call finishes (subject to the
  300s HTTP client ceiling).
- **Set to N:** wait up to N seconds. If still running, return immediately:

```json
{
  "ok": true,
  "status": "running",
  "job_id": "job_…",
  "elapsed_secs": N,
  "next": "call job_status with job_id=… until status is completed or failed"
}
```

Then poll `job_status` until `status` is `completed` (full result in `result`) or
`failed`. Jobs are **in-memory** (lost on process restart); finished jobs expire
after ~30 minutes. Max **2** concurrent background jobs.

### Phase 2 (out of scope for this spec revision)

Documented only so Phase 1 names do not collide later: `web_search`, `code_execution`, `deep_research` / `multi_agent`, `generate_image`, `analyze_image`.

---

## 1. `research`

Run a research task on xAI. The server instructs Grok to use native tools when
`sources` includes them, then **collapses** the trajectory into a digest for the host.

### Input

```json
{
  "properties": {
    "query": {
      "type": "string",
      "description": "Research question or topic. Non-empty after trim; max 4000 chars."
    },
    "sources": {
      "type": "array",
      "items": { "type": "string", "enum": ["web", "x"] },
      "default": ["web"],
      "description": "Native xAI tools to enable. Empty array = no live search (model knowledge only)."
    },
    "verbosity": {
      "type": "string",
      "enum": ["summary", "detailed", "raw"],
      "default": "summary"
    },
    "model": { "type": "string" },
    "reasoning_effort": {
      "type": "string",
      "enum": ["low", "medium", "high"],
      "default": "medium"
    },
    "max_output_tokens": {
      "type": "integer",
      "minimum": 64,
      "maximum": 8192,
      "default": 2048
    }
  },
  "required": ["query"]
}
```

### Output (success)

```json
{
  "ok": true,
  "answer": "Dense answer paragraph(s).",
  "key_points": ["…"],
  "sources": [
    { "title": "…", "url": "https://…", "kind": "web" }
  ],
  "confidence": "low|medium|high",
  "model": "grok-4.5",
  "usage": {
    "input_tokens": 0,
    "output_tokens": 0,
    "reasoning_tokens": 0
  },
  "truncated": false,
  "raw": null
}
```

- `sources` entries are **citations only** (title + url + kind). No full page bodies.
- `raw` is present only when `verbosity=raw` (upstream text / tool traces, size-capped).
- `usage` is best-effort from the Responses API; omit fields if upstream does not provide them (still include the object with nulls or partials).

### Semantics

- Empty / whitespace `query` → `INVALID_PARAMS`.
- Duplicate entries in `sources` are deduped.
- Unknown `model` → `INVALID_PARAMS` with allowed list in `details`.

---

## 2. `x_search`

Search X via xAI's native `x_search` tool, wrapped so the host gets a digest
instead of an unbounded post dump.

### Input

```json
{
  "properties": {
    "query": {
      "type": "string",
      "description": "X search query. Non-empty after trim; max 1000 chars."
    },
    "verbosity": {
      "type": "string",
      "enum": ["summary", "detailed", "raw"],
      "default": "summary"
    },
    "model": { "type": "string" },
    "max_results": {
      "type": "integer",
      "minimum": 1,
      "maximum": 20,
      "default": 8,
      "description": "Soft target for number of posts/citations to include in the digest."
    },
    "max_output_tokens": {
      "type": "integer",
      "minimum": 64,
      "maximum": 4096,
      "default": 1024
    }
  },
  "required": ["query"]
}
```

### Output (success)

```json
{
  "ok": true,
  "summary": "What the posts collectively say.",
  "posts": [
    {
      "author": "@handle",
      "text": "Short excerpt or full short post",
      "url": "https://x.com/…",
      "engagement_hint": "optional free text if known"
    }
  ],
  "model": "…",
  "usage": { "input_tokens": 0, "output_tokens": 0 },
  "truncated": false,
  "raw": null
}
```

- Default path asks Grok to return at most `max_results` items with short excerpts.
- `verbosity=raw` may include larger upstream fragments under `raw`, still capped.

---

## 3. `ask_grok`

Single-shot offload: Q&A, code critique, design review, rewrite. No native
search tools unless the server later adds an explicit flag (Phase 1: **none**).

### Input

```json
{
  "properties": {
    "prompt": {
      "type": "string",
      "description": "User task for Grok. Non-empty after trim; max 100_000 chars (server may reject if over global request budget)."
    },
    "system": {
      "type": "string",
      "description": "Optional system/instructions prefix. Max 8_000 chars."
    },
    "verbosity": {
      "type": "string",
      "enum": ["summary", "detailed", "raw"],
      "default": "summary",
      "description": "summary = concise answer; detailed = fuller write-up; raw = minimal post-processing of model text (still size-capped)."
    },
    "model": { "type": "string" },
    "reasoning_effort": {
      "type": "string",
      "enum": ["low", "medium", "high"],
      "default": "medium"
    },
    "max_output_tokens": {
      "type": "integer",
      "minimum": 64,
      "maximum": 8192,
      "default": 2048
    }
  },
  "required": ["prompt"]
}
```

### Output (success)

```json
{
  "ok": true,
  "text": "Grok's answer (post-processed per verbosity).",
  "model": "…",
  "usage": {
    "input_tokens": 0,
    "output_tokens": 0,
    "reasoning_tokens": 0
  },
  "truncated": false
}
```

### Semantics

- Intended for **expensive host work**: long critique, alternative design, second opinion.
- Host agents should pass **only the material needed** (not entire repos). Server may reject oversized `prompt` with `INVALID_PARAMS`.
- Phase 1: no multi-turn `previous_response_id` chaining.

---

## 4. `auth_status`

Report whether the server can call xAI. **Never** returns access tokens, refresh
tokens, or API keys.

### Input

```json
{
  "properties": {
    "include_account_hints": {
      "type": "boolean",
      "default": true,
      "description": "If true, include non-secret account hints (email local-part redaction policy: full email allowed if already in local auth file; implementations may redact)."
    }
  }
}
```

### Output (success)

```json
{
  "ok": true,
  "authenticated": true,
  "billing_path": "subscription_oauth",
  "source": "grok-cli|device_code|api_key|none",
  "expires_at": "2026-07-12T12:00:00Z",
  "expires_in_secs": 3600,
  "account": {
    "email": "user@example.com",
    "user_id": "…"
  },
  "api_key_opt_in": false,
  "api_key_present": false,
  "last_error": null
}
```

| Field | Notes |
|---|---|
| `billing_path` | `"subscription_oauth"` \| `"api_key"` \| `"none"` — path that **would** be used for the next request |
| `source` | Where credentials were loaded from |
| `api_key_opt_in` | Whether `GROK_MCP_ALLOW_API_KEY` (or equivalent) is enabled |
| `api_key_present` | Whether a key is configured (boolean only) |
| `last_error` | Last persisted auth failure code/message if any, else null |

This tool does not network to xAI unless a lightweight validate mode is added later; Phase 1 is **local file + expiry math only**.

---

## Auth CLI (not MCP tools)

End-user / operator commands on the binary (names fixed for README):

| Command | Behavior |
|---|---|
| `grok-mcp auth status` | Same data as `auth_status` tool, human-readable |
| `grok-mcp auth login` | Device-code OAuth; writes `~/.config/grok-mcp/auth.json` |
| `grok-mcp auth import` | Import from `~/.grok/auth.json` into the grok-mcp store |
| `grok-mcp auth logout` | Delete grok-mcp store only (never modifies `~/.grok/auth.json`) |

Credential resolution order is defined in [ADR-0003](adr/0003-xai-authentication.md).

---

## Configuration surface (normative names)

| Variable / flag | Purpose |
|---|---|
| `GROK_MCP_AUTH_FILE` | Override path to auth store |
| `GROK_MCP_ALLOW_API_KEY` | `1`/`true` to allow API key billing path |
| `XAI_API_KEY` | Pay-per-token key; used only if allow opt-in is on |
| `GROK_MCP_DEFAULT_MODEL` | Default model id |
| `GROK_MCP_BASE_URL` | Default `https://api.x.ai/v1` |
| `--stdio` / `--http` | Transport selection (see ADR-0005) |

---

## Host guidance (for tool descriptions)

Each tool's MCP `description` should tell the host model:

1. Prefer these tools when the task would burn **many Claude turns** (research, X monitoring, long critique).
2. Trust the digest within stated `confidence`; do not re-run the same search with host-native tools unless verification is required.
3. Keep `verbosity=summary` unless the user asked for depth.
4. On `REAUTH_REQUIRED`, tell the user to run `grok-mcp auth login` — do not invent credentials.

---

## Non-goals (Phase 1)

- Exposing a raw Responses proxy (`respond` tool)
- Server-side conversation memory / session ids across MCP calls
- Streaming token deltas back through MCP (return final JSON only)
- Image / video / TTS tools
- Multi-tenant credential pools
- Mutating `~/.grok/auth.json` in place
