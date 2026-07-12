#!/usr/bin/env bash
# Load .local/env (or GROK_MCP_ENV_FILE) and start Streamable HTTP in the foreground.
#
#   ./deploy/init-local-env.sh
#   ./deploy/run-local.sh
set -euo pipefail

REPO="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
ENV_FILE="${GROK_MCP_ENV_FILE:-$REPO/.local/env}"
BIN="${GROK_MCP_BIN:-}"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  echo "run: $REPO/deploy/init-local-env.sh" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

if [[ -z "${GROK_MCP_HTTP_TOKEN:-}" || "$GROK_MCP_HTTP_TOKEN" == "REPLACE_ME" ]]; then
  echo "GROK_MCP_HTTP_TOKEN is unset or REPLACE_ME — edit $ENV_FILE" >&2
  exit 1
fi

if [[ -z "$BIN" ]]; then
  if [[ -x "$REPO/target/release/grok-mcp" ]]; then
    BIN="$REPO/target/release/grok-mcp"
  elif [[ -x "$REPO/target/debug/grok-mcp" ]]; then
    BIN="$REPO/target/debug/grok-mcp"
  elif command -v cargo >/dev/null 2>&1; then
    echo "building release binary…"
    (cd "$REPO" && cargo build --release -p grok-server --quiet)
    BIN="$REPO/target/release/grok-mcp"
  else
    echo "no grok-mcp binary found; build with: cargo build --release -p grok-server" >&2
    exit 1
  fi
fi

if [[ ! -f "${HOME}/.config/grok-mcp/auth.json" ]]; then
  echo "warning: no ~/.config/grok-mcp/auth.json — run: $BIN auth import  (or auth login)" >&2
fi

echo "public host    : ${GROK_MCP_HTTP_ALLOWED_HOSTS:-'(set in env)'}"
echo "local bind     : ${GROK_MCP_HTTP_ADDR:-127.0.0.1:8765}"
echo "token (first 8): ${GROK_MCP_HTTP_TOKEN:0:8}…"
echo "binary         : $BIN"
echo "— Ctrl-C to stop —"

exec "$BIN" --http
