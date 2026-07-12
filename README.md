# grok-mcp

MCP server that lets **Claude** (and other MCP clients) use your **xAI / SuperGrok
subscription** for research, X search, and heavy analysis — so those jobs burn
Grok quota instead of Claude tokens.

It calls the xAI **Responses API** with a SuperGrok OAuth session (or an
opt-in pay-per-token `XAI_API_KEY`). Tool results are **dense digests by
default**, not raw dumps.

| | |
|---|---|
| **Language** | Rust (rmcp) |
| **Transports** | stdio · Streamable HTTP (`/mcp`) |
| **Auth (xAI)** | `auth import` / device-code `auth login` + refresh |
| **Auth (HTTP front-door)** | Optional bearer + co-hosted OAuth 2.1 (claude.ai) |

Schemas: [`docs/tool_spec.md`](docs/tool_spec.md) · ADRs: [`docs/adr/`](docs/adr/).

## Tools

| Tool | Role |
|---|---|
| `ask_grok` | Low-cost Q&A / critique / analysis — **no** web/X search |
| `x_search` | X (Twitter) only via native `x_search` |
| `research` | Multi-step web/X research (higher SuperGrok quota use) |
| `job_status` | Poll jobs started with `timeout_secs` |
| `auth_status` | Non-secret xAI login health |

Shared generative options: `verbosity` (`summary` \| `detailed` \| `raw`),
`reasoning_effort` (`low` \| `medium` \| `high` where supported),
`max_output_tokens`, optional **`timeout_secs` (1–300)**.

If `timeout_secs` elapses before the call finishes, the tool returns
`status: "running"` + `job_id` — poll **`job_status`** until `completed` or
`failed`. Jobs are in-memory (lost on restart); max 2 concurrent.

## Requirements

- **SuperGrok** or **X Premium+** linked to xAI (subscription OAuth), **or**
- `XAI_API_KEY` **and** `GROK_MCP_ALLOW_API_KEY=1` (off by default)
- Rust 1.95+ to build from source

Optional: reuse **Grok CLI / Grok Build** credentials via `auth import`
(`~/.grok/auth.json`).

## Quick start (stdio)

Grab a binary from [GitHub Releases](https://github.com/zekaizer/grok-mcp/releases)
(Linux is the primary asset; Windows is best-effort), or build from source:

```sh
cargo build --release -p grok-server
# binary: target/release/grok-mcp

# SuperGrok session
./target/release/grok-mcp auth import   # or: auth login
./target/release/grok-mcp auth status

# Claude Code
claude mcp add --scope project grok-xai -- \
  "$(pwd)/target/release/grok-mcp" --stdio

# Grok Build
grok mcp add --scope project grok-xai -- \
  "$(pwd)/target/release/grok-mcp" --stdio
```

## Production (systemd + Cloudflare Tunnel)

Host-specific secrets and hostnames live under **`.local/`** (not in git).
See [`deploy/README.md`](deploy/README.md).

```sh
./deploy/init-local-env.sh          # → .local/env + .local/deploy.env
# edit .local/* for your public host, tunnel id, token

cargo build --release -p grok-server
./target/release/grok-mcp auth import

sudo ./deploy/grokctl bootstrap
sudo ./deploy/grokctl status
```

```sh
TOKEN=$(sudo sed -n 's/^GROK_MCP_HTTP_TOKEN=//p' /etc/grok-mcp/env)
# use the hostname you put in .local/deploy.env
claude mcp add --transport http grok-xai https://YOUR_HOST/mcp \
  --header "Authorization: Bearer ${TOKEN}"
```

## Configuration (selected)

| Variable | Purpose |
|---|---|
| `GROK_MCP_AUTH_FILE` | xAI credential store path |
| `GROK_MCP_DEFAULT_MODEL` | Default model (e.g. `grok-4.5`) |
| `GROK_MCP_BASE_URL` | Default `https://api.x.ai/v1` |
| `GROK_MCP_ALLOW_API_KEY` / `XAI_API_KEY` | Opt-in pay-per-token path |
| `GROK_MCP_TRANSPORT` | `stdio` \| `http` |
| `GROK_MCP_HTTP_ADDR` | Bind (default `127.0.0.1:8765`) |
| `GROK_MCP_HTTP_TOKEN` | Front-door bearer + OAuth gate |
| `GROK_MCP_HTTP_ALLOWED_HOSTS` | Host allowlist (public hostname for tunnels) |
| `GROK_MCP_STATE_DIR` | OAuth issued-token store |

Machine-only deploy overrides (not committed): `.local/deploy.env`, `.local/env`.

`grok-mcp --help` lists all flags.

## Security

- xAI tokens stay on the **server** only; MCP clients never see them.
- Do not expose HTTP without a front-door token.
- Do **not** put Cloudflare Access in front of the MCP hostname — it breaks
  the claude.ai OAuth connector.
- SuperGrok OAuth may return **403** on some accounts (entitlement); the server
  reports `ENTITLEMENT_DENIED` and does not silently fall back to API keys.

## Troubleshooting

| Symptom | What to try |
|---|---|
| `REAUTH_REQUIRED` | `grok-mcp auth login` or `auth import` |
| `ENTITLEMENT_DENIED` | Check SuperGrok / X Premium+; then API key opt-in if needed |
| `RATE_LIMITED` | Back off; avoid stacking `research` |
| `status=running` forever | `job_status`; process restart clears in-memory jobs |
| Claude never calls tools | Name tools explicitly; check MCP connection / approval |

## Development

```sh
make check          # fmt + clippy -D warnings + tests
cargo test --workspace
```

| Doc | Contents |
|---|---|
| [`docs/tool_spec.md`](docs/tool_spec.md) | Normative tool schemas |
| [`docs/adr/`](docs/adr/) | Architecture decisions |
| [`deploy/`](deploy/) | systemd, env sample, `grokctl`, cloudflared |

## License

[MIT](LICENSE)
