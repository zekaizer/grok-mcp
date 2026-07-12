#!/usr/bin/env bash
# Create .local/env from the sample and fill a random front-door token.
# Machine-local only (see .git/info/exclude — never commit).
set -euo pipefail

REPO="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
SAMPLE="$REPO/deploy/grok-mcp.env.example"
OUT_DIR="$REPO/.local"
OUT="$OUT_DIR/env"
SHOW=0

for arg in "$@"; do
  case "$arg" in
    --show-token) SHOW=1 ;;
    -h|--help)
      echo "usage: $0 [--show-token]"
      exit 0
      ;;
  esac
done

if [[ ! -f "$SAMPLE" ]]; then
  echo "missing sample $SAMPLE" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
TOKEN="$(head -c 32 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=')"

if [[ -f "$OUT" ]]; then
  if grep -q '^GROK_MCP_HTTP_TOKEN=REPLACE_ME' "$OUT" 2>/dev/null; then
    sed -i "s|^GROK_MCP_HTTP_TOKEN=REPLACE_ME|GROK_MCP_HTTP_TOKEN=${TOKEN}|" "$OUT"
    echo "updated token in $OUT"
  else
    echo "exists $OUT (token left unchanged; delete file to regenerate from sample)"
  fi
else
  cp "$SAMPLE" "$OUT"
  sed -i "s|^GROK_MCP_HTTP_TOKEN=REPLACE_ME|GROK_MCP_HTTP_TOKEN=${TOKEN}|" "$OUT"
  # Prefer $HOME in state path
  sed -i "s|/home/YOUR_USER|${HOME}|g" "$OUT"
  chmod 0600 "$OUT"
  echo "wrote $OUT (mode 0600)"
fi

if [[ ! -f "$OUT_DIR/deploy.env" ]]; then
  cat >"$OUT_DIR/deploy.env" <<EOF
# Host-only overrides for deploy/grokctl (not committed)
# GROK_MCP_PUBLIC_HOST=mcp.example.com
# GROK_MCP_CF_TUNNEL_ID=<uuid>
# GROK_MCP_HTTP_ADDR=127.0.0.1:8765
# GROK_MCP_CF_CONFIG=${OUT_DIR}/cloudflared-config.yml
EOF
  chmod 0600 "$OUT_DIR/deploy.env"
  echo "wrote $OUT_DIR/deploy.env (edit host/tunnel values)"
fi

echo
echo "Next:"
echo "  1) Edit .local/env and .local/deploy.env for your host"
echo "  2) SuperGrok:  cargo run -p grok-server -- auth import"
echo "  3) Server:     ./deploy/run-local.sh"
echo "  4) Tunnel:     copy deploy/cloudflared-config.example.yml → .local/cloudflared-config.yml"
echo "  5) Production: sudo ./deploy/grokctl bootstrap"
echo

if [[ "$SHOW" -eq 1 ]]; then
  echo "GROK_MCP_HTTP_TOKEN=$(grep '^GROK_MCP_HTTP_TOKEN=' "$OUT" | cut -d= -f2-)"
else
  echo "Token is in $OUT (use --show-token to print)."
fi
