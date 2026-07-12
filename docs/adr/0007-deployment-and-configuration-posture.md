# 7. Deployment: systemd + a single env file

## Status

Accepted

Builds on [ADR-0005](0005-transport-and-deployment-phases.md) (phased transport)
and [ADR-0006](0006-http-transport-and-front-door-auth.md) (HTTP + front-door
OAuth). Pattern aligned with md-mcp ADR-0020.

## Context

grok-mcp is moving from a local stdio testbed to an always-on personal
deployment: one Linux host, SuperGrok credentials on disk, exposed through a
Cloudflare Tunnel to claude.ai and Claude Code. We need process supervision and
a configuration surface that does not invent a second format.

Facts:

- Configuration is **flat scalars** (bind addr, token, host allowlist, paths,
  model defaults). No nesting, no multi-tenant policy.
- systemd consumes env files natively (`EnvironmentFile=`); the dev loop can
  `source` the same file.
- **xAI secrets must not live in the env file** — they already have a dedicated
  store (`~/.config/grok-mcp/auth.json`) with refresh (ADR-0003). The env file
  only holds the **MCP front-door** token and operational knobs.
- Secrets protection is file mode `0600`, not format choice.

## Decision

We will deploy as a **systemd system service** reading **one env file**
(`/etc/grok-mcp/env`, mode `0600`), fronted by **cloudflared** as its own
service. We will **not** introduce `config.toml`.

Sample assets live in `deploy/`:

| Asset | Role |
|---|---|
| `grok-mcp.service` | Hardened unit (`ProtectSystem=strict`, explicit `ReadWritePaths=`) |
| `grok-mcp.env.example` | Annotated production env (HTTP + front-door token) |
| `grokctl` | install / update / rollback / health / logs / token |
| `README.md` | Step-by-step (binary, env, tunnel, clients) |

Posture defaults in the sample:

- `GROK_MCP_TRANSPORT=http` / loopback bind `127.0.0.1:8766`
- `GROK_MCP_LOG_FORMAT=json`
- Tunnel hostname in `GROK_MCP_HTTP_ALLOWED_HOSTS`
- `GROK_MCP_HTTP_TOKEN` required for production health (401 probe)
- Explicit `GROK_MCP_STATE_DIR` for OAuth persistence under `ReadWritePaths=`

Revisit triggers for a config file: multi-tenant front-door users, nested
policy, or the flat surface outgrowing ~25 keys.

## Consequences

- Positive: crash restart, boot persistence, journald; zero new config parser in
  the server; one file to back up for front-door secrets.
- Positive: SuperGrok tokens stay in the auth store, not the env file.
- Negative: env files carry no structure; quoting must stay dual-compatible with
  shell and systemd (documented in `deploy/README.md`).
- Neutral: containers can reuse the same env file (`docker run --env-file`).
