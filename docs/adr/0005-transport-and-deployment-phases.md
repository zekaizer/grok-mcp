# 5. Transport and deployment phases

## Status

Accepted

## Context

Two clients matter:

1. **Claude Code (primary)** — can launch a local process over **stdio** or
   connect to HTTP MCP with headers.
2. **claude.ai custom connector (secondary)** — Anthropic's cloud opens a
   **public HTTPS** Streamable HTTP MCP endpoint and typically completes
   **OAuth 2.1 + PKCE** against the MCP server (front-door), not against xAI.

Building remote auth, public exposure, and dual transport on day one delays the
tooling that proves token offload. Conversely, designing stdio-only forever
would force a rewrite for claude.ai.

## Decision

We will ship transport in two phases on **one binary** and one tool surface.

### Phase A — local stdio (first)

- Default developer path: `grok-mcp --stdio` (or stdio when launched by the client).
- xAI credentials from ADR-0003 on the **same machine** as Claude Code.
- No public network listener required.
- Success metric: `research` / `x_search` / `ask_grok` reduce host multi-turn work
  in real Claude Code sessions.

### Phase B — Streamable HTTP + remote connector

- Same tools; add `--http` (bind address/port configurable).
- MCP transport: **Streamable HTTP** only (not SSE-primary).
- **MCP front-door authentication** required before exposing beyond loopback:
  OAuth 2.1 with PKCE and discovery metadata suitable for claude.ai custom
  connectors. This is independent of xAI OAuth.
- xAI tokens remain on the server host (VPS or home server); operators run
  `grok-mcp auth login` once on that host (device-code).
- `rmcp` HTTP stack must be a patched version for known Host-header issues
  (≥ 1.4.0).

### Explicit non-goals until Phase B design spike

- Multi-tenant per-user xAI credential pools.
- Exposing the HTTP port on `0.0.0.0` without front-door auth.

## Consequences

- Positive: Phase A delivers user value with minimal attack surface.
- Positive: tool_spec stays stable across phases; only transport and front-door auth grow.
- Negative: Phase B is a substantial security feature (OAuth AS or integration),
  not a flag flip.
- Negative: remote deployment means the operator's SuperGrok session is a
  high-value secret on that server.
- Neutral: Claude Code may later use HTTP to a local Phase B server; stdio remains supported.
