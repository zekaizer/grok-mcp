# 3. xAI authentication and credential storage

## Status

Accepted

## Context

The product goal is to spend **SuperGrok / X Premium+ subscription quota**
instead of Claude tokens and instead of pay-per-token xAI API spend. xAI exposes
that quota through OAuth bearer tokens used against `https://api.x.ai/v1`.

Existing local artifacts on developer machines:

- `~/.grok/auth.json` — Grok CLI / Grok Build OIDC store (`refresh_token`,
  `oidc_client_id`, `expires_at`, …).
- Hermes and other agents may keep separate device-code or PKCE tokens.

Threat model:

- MCP hosts (including Anthropic cloud for remote connectors) must **never**
  receive xAI refresh or access tokens.
- For remote deployment, the server process holds the subscription session;
  a separate **MCP front-door** auth (Phase B) gates who may invoke tools.
- Silent fallback to `XAI_API_KEY` would burn real money while the user believes
  they are on subscription quota.

## Decision

### Credential resolution order

At request time we will resolve credentials in this order:

1. `GROK_MCP_AUTH_FILE` if set (test / remote explicit path).
2. Default grok-mcp store: `~/.config/grok-mcp/auth.json` (or platform equivalent via the `directories` crate).
3. Import candidate: `~/.grok/auth.json` when the grok-mcp store is missing or empty — **copy-on-import** into the grok-mcp store, do not mutate the CLI file for refresh bookkeeping.
4. Optional later: other known agent stores (e.g. Hermes) — not required for Phase 1.
5. If still none: fail with `REAUTH_REQUIRED` and instruct `grok-mcp auth login`.

### Login and storage

- **Primary zero-touch path:** reuse `~/.grok/auth.json` via import.
- **Primary interactive path:** **OAuth 2.0 device-code** against xAI
  (`auth.x.ai` / documented device endpoints). No loopback PKCE requirement for
  Phase 1 (headless- and VPS-friendly).
- **Store format:** versioned JSON owned by grok-mcp (`version`, `source`,
  tokens, `expires_at`, OIDC client metadata). File mode `0600`, atomic write
  (temp + rename), optional lock file for concurrent refresh.
- **Refresh:** update only the grok-mcp store. Never rewrite `~/.grok/auth.json`
  in place.
- **Logout:** delete the grok-mcp store only.

### API key path (opt-in only)

We will support `XAI_API_KEY` **only when** `GROK_MCP_ALLOW_API_KEY` is truthy
(`1` / `true` / `yes`). Otherwise the key is ignored even if present.

- Default: opt-in **off**.
- `auth_status` reports `api_key_opt_in`, `api_key_present`, and which
  `billing_path` would be used.
- On subscription OAuth `403` entitlement errors we will **not** auto-switch to
  the API key. The error is `ENTITLEMENT_DENIED` with guidance.

### Secret exposure

- MCP tool `auth_status` and CLI `auth status` never print access/refresh tokens
  or API key material.
- Logs redact `Authorization` headers and token fields.

## Consequences

- Positive: Grok CLI users run tools without a second login.
- Positive: device-code works for Phase B servers without browser loopback.
- Positive: no silent paid API fallback.
- Negative: two files can diverge after import (CLI vs grok-mcp); user may need
  `auth import` or `auth login` after CLI re-login.
- Negative: device-code client id / endpoint details depend on xAI's public OAuth
  surface and may need updates if xAI changes apps.
- Neutral: MCP front-door OAuth for claude.ai is **out of scope for this ADR**
  (see ADR-0005); it authenticates the *caller of grok-mcp*, not xAI.
