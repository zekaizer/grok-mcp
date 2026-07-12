# Deploying grok-mcp (ADR-0007)

Host-specific values (**hostname, tunnel id, tokens, home paths**) live under
**`.local/`** (excluded from git via `.git/info/exclude`). Tracked files only
contain placeholders.

| Asset | Role |
|---|---|
| [`grok-mcp.env.example`](grok-mcp.env.example) | Env template |
| [`cloudflared-config.example.yml`](cloudflared-config.example.yml) | Tunnel ingress template |
| [`init-local-env.sh`](init-local-env.sh) | Create `.local/env` + `.local/deploy.env` |
| [`run-local.sh`](run-local.sh) | Source `.local/env` and run HTTP server |
| [`grokctl`](grokctl) | systemd install / update / health |
| [`grok-mcp.service`](grok-mcp.service) | systemd unit template |

---

## One-shot (systemd + Cloudflare Tunnel)

```sh
# 1) Private host config (not committed)
./deploy/init-local-env.sh
# edit .local/env          — token, GROK_MCP_HTTP_ALLOWED_HOSTS, paths
# edit .local/deploy.env   — GROK_MCP_PUBLIC_HOST, GROK_MCP_CF_TUNNEL_ID, addr
# copy deploy/cloudflared-config.example.yml → .local/cloudflared-config.yml and fill

# 2) SuperGrok as the service user
cargo build --release -p grok-server
./target/release/grok-mcp auth import   # or auth login

# 3) Install
sudo ./deploy/grokctl bootstrap
sudo ./deploy/grokctl status
```

`grokctl` reads `.local/deploy.env` when present.

```sh
TOKEN=$(sudo sed -n 's/^GROK_MCP_HTTP_TOKEN=//p' /etc/grok-mcp/env)
claude mcp add --transport http grok-xai "https://${GROK_MCP_PUBLIC_HOST}/mcp" \
  --header "Authorization: Bearer ${TOKEN}"
```

**Do not** put Cloudflare Access on the MCP hostname — it breaks the claude.ai
OAuth connector (ADR-0006).

---

## Dev path (no systemd)

```sh
./deploy/init-local-env.sh
./deploy/run-local.sh
```

---

## Quoting / secrets

- Never commit `.local/` or `/etc/grok-mcp/env`.
- SuperGrok tokens stay in `~/.config/grok-mcp/auth.json`, not the env file.
- Front-door token is only for MCP clients (Claude / Grok HTTP).

## Updates

```sh
sudo ./deploy/grokctl update
sudo ./deploy/grokctl rollback   # if needed
```
