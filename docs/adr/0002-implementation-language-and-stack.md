# 2. Implementation language and stack

## Status

Accepted

## Context

grok-mcp must:

1. Speak MCP over **stdio** (Claude Code) and later **Streamable HTTP** (claude.ai).
2. Perform OAuth device-code login, token refresh, and careful file storage.
3. Call xAI REST endpoints (`/v1/responses` and related).
4. Run as a long-lived local or VPS process with a single deployable artifact.

The maintainer already operates **md-mcp**, a Rust MCP server built on official
**rmcp**, tokio, axum, and clap. Reusing that stack reduces learning cost and
lets patterns (error envelopes, dual transport, OAuth front-door for Phase B)
transfer.

Alternative stacks (TypeScript with the official MCP SDK, Python) would make it
easier to copy logic from pi-xai-oauth or Hermes, but those projects are
*provider plugins*, not standalone dual-transport MCP servers. The hard parts
here are server lifecycle, auth isolation, and output caps — not model SDKs.

## Decision

We will implement grok-mcp in **Rust** (edition aligned with md-mcp / current
stable toolchain pinned by `rust-toolchain.toml`).

### Core dependencies

| Concern | Choice |
|---|---|
| MCP SDK | **rmcp** ≥ 1.4 (prefer same minor line as md-mcp, e.g. 1.8), features: `server`, `macros`, `schemars`, `transport-io`, and later `transport-streamable-http-server` |
| Async runtime | **tokio** |
| HTTP client (xAI) | **reqwest** (rustls), JSON via serde |
| CLI | **clap** |
| Logging | **tracing** + **tracing-subscriber** (stderr; stdout reserved for stdio JSON-RPC) |
| Serialization | **serde** / **serde_json** |

### Workspace layout

```text
crates/
  grok-auth/    # credential resolve, device-code, refresh, store I/O
  grok-client/  # Responses client, model allowlist, dense post-processing
  grok-server/  # MCP tools + binary (stdio / HTTP)
```

We will **not** depend on an official xAI Rust SDK unless one becomes clearly
maintained and smaller than a thin REST client. Responses payloads stay explicit
in `grok-client`.

### What we will not do

- Embed a JavaScript runtime or shell out to pi/Hermes for inference.
- Use SSE as the primary remote transport (Streamable HTTP only for Phase B).

## Consequences

- Positive: single static binary; shared operational DNA with md-mcp; rmcp supports stdio and Streamable HTTP.
- Positive: no GC pauses for a long-running token-refresh process.
- Negative: cannot paste TypeScript OAuth snippets from pi-xai-oauth; we reimplement against the same OAuth endpoints.
- Negative: slightly slower greenfield iteration than a scripting language.
- Neutral: end-to-end tests can follow md-mcp's style (Rust unit + optional Python harness later).
