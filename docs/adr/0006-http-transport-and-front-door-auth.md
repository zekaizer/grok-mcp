# 6. Streamable HTTP transport and MCP front-door auth

## Status

Accepted

Implements the Phase B half of [ADR-0005](0005-transport-and-deployment-phases.md).

## Context

Phase A is stdio for Claude Code / Grok Build. Phase B needs:

1. **Streamable HTTP** so remote clients (claude.ai custom connector, shared
   VPS) can reach the same tool surface.
2. **Front-door authentication** that is *not* the SuperGrok token: MCP clients
   must not receive xAI credentials. Claude Code can send a static bearer;
   claude.ai speaks **OAuth 2.1 + PKCE** only.

The sibling project md-mcp already proved Streamable HTTP via rmcp + axum and a
co-hosted OAuth 2.1 AS gated by a static ownership token.

## Decision

We will:

1. Enable rmcp `transport-streamable-http-server` and serve
   `StreamableHttpService` with **axum** at path `/mcp`.
2. Default bind **`127.0.0.1:8765`**. Non-loopback binds warn; there is no
   in-process TLS (terminate TLS at a reverse proxy / tunnel).
3. **Static bearer** via `GROK_MCP_HTTP_TOKEN` or `--http-token-file` (never as a
   bare argv value when avoidable). When the token is set:
   - `/mcp` requires `Authorization: Bearer …` (static token **or** issued
     OAuth access token).
   - A **co-hosted OAuth 2.1 AS** is mounted (DCR, authorize + PKCE S256, token,
     refresh, RFC 9728 / RFC 8414 metadata), using the static token as the
     human ownership gate on `/authorize`.
4. Host and Origin allowlists default to loopback; `*` disables.
5. State (registered clients + issued tokens) lives under
   `GROK_MCP_STATE_DIR` (default `$XDG_STATE_HOME/grok-mcp`).

stdio remains the default transport when neither `--http` nor
`GROK_MCP_TRANSPORT=http` is set.

## Consequences

- Positive: Claude Code can use HTTP + bearer; claude.ai can complete OAuth
  against the same origin when the server is publicly reachable over HTTPS.
- Positive: SuperGrok tokens stay server-side only.
- Negative: Co-hosted AS is single-user / ownership-token gated — not a multi-tenant IdP.
- Negative: Public deployment still requires an external TLS terminator and careful Host allowlisting.
- Neutral: Pattern and much of the OAuth code are aligned with md-mcp for maintainability.
