#!/usr/bin/env bash
# Raw-request capture script for the `create` tool's image API call.
#
# Mirrors ImageApiClient::generate() in src/image_api.rs exactly:
#   POST {base_url}/v1/images/generations
#   Content-Type: application/json
#   Authorization: Bearer <api_key>
#   {
#     "prompt": <prompt>,
#     "model": <model>,
#     "n": <n>,
#     "size": <size>,
#     "output_format": <format>,
#     "response_format": "b64_json"
#   }
#
# Params resolve the same way ImageParams::resolve() does in
# src/tools/mod.rs: CLI flags override config's `create_defaults`, unmet
# flags fall back to the config default for each field.
#
# Every raw byte sent/received on the wire is captured via curl's
# --trace-ascii into the capture directory, alongside a resolved-params
# summary and the parsed image outputs (if any).
#
# Usage:
#   ./create.sh --prompt "a red bicycle" [--model gpt-image-1] [--n 1] \
#               [--size 1024x1024] [--format png]
#
# Config: reads ~/.config/image-mcp/config.json (override with
# IMAGE_MCP_CONFIG=/path/to/config.json).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

require_cmd curl
require_cmd jq
require_cmd python3

PROMPT=""
MODEL=""
N=""
SIZE=""
FORMAT=""

usage() {
    cat <<EOF
Usage: $0 --prompt "<text>" [--model MODEL] [--n N] [--size WxH] [--format png|jpg|webp]
EOF
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prompt) PROMPT="$2"; shift 2 ;;
        --model) MODEL="$2"; shift 2 ;;
        --n) N="$2"; shift 2 ;;
        --size) SIZE="$2"; shift 2 ;;
        --format) FORMAT="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "unknown argument: $1" >&2; usage ;;
    esac
done

[ -n "$PROMPT" ] || { echo "error: --prompt is required" >&2; usage; }

CONFIG_JSON="$(load_config_json)"

BASE_URL="$(cfg "$CONFIG_JSON" '.image_api.base_url' | sed 's:/*$::' | sed 's:/v1/*$::')"
API_KEY="$(cfg "$CONFIG_JSON" '.image_api.api_key')"

# Resolve against create_defaults, same precedence as ImageParams::resolve.
RESOLVED_MODEL="${MODEL:-$(cfg "$CONFIG_JSON" '.create_defaults.model')}"
RESOLVED_N="${N:-$(cfg "$CONFIG_JSON" '.create_defaults.n')}"
RESOLVED_SIZE="${SIZE:-$(cfg "$CONFIG_JSON" '.create_defaults.size')}"
RESOLVED_FORMAT="${FORMAT:-$(cfg "$CONFIG_JSON" '.create_defaults.format')}"

URL="$BASE_URL/v1/images/generations"

BODY="$(jq -nc \
    --arg prompt "$PROMPT" \
    --arg model "$RESOLVED_MODEL" \
    --argjson n "$RESOLVED_N" \
    --arg size "$RESOLVED_SIZE" \
    --arg output_format "$RESOLVED_FORMAT" \
    '{prompt: $prompt, model: $model, n: $n, size: $size, output_format: $output_format, response_format: "b64_json"}')"

DIR="$(new_capture_dir create)"
echo "capturing to: $DIR"

cat >"$DIR/meta.json" <<EOF
{
  "endpoint": "$URL",
  "method": "POST",
  "resolved_params": {
    "prompt": $(jq -n --arg p "$PROMPT" '$p'),
    "model": "$RESOLVED_MODEL",
    "n": $RESOLVED_N,
    "size": "$RESOLVED_SIZE",
    "format": "$RESOLVED_FORMAT"
  }
}
EOF

echo "$BODY" | jq . >"$DIR/request-body.json"

# --trace-ascii captures the literal bytes sent/received on the wire
# (headers + body, both directions) for later low-level analysis.
HTTP_STATUS="$(curl -sS \
    --trace-ascii "$DIR/wire-trace.txt" \
    -o "$DIR/response-body.raw" \
    -D "$DIR/response-headers.txt" \
    -w '%{http_code}' \
    -X POST "$URL" \
    -H "Authorization: Bearer $API_KEY" \
    -H "Content-Type: application/json" \
    --data "$BODY")"

echo "$HTTP_STATUS" >"$DIR/response-status.txt"
echo "HTTP status: $HTTP_STATUS"

# Redact the bearer token from the trace/headers copies kept for review,
# leave an unredacted copy only if you need to replay the exact request.
sed "s/Authorization: Bearer .*/Authorization: Bearer [REDACTED]/" "$DIR/wire-trace.txt" >"$DIR/wire-trace.redacted.txt"

if jq -e . "$DIR/response-body.raw" >"$DIR/response-body.json" 2>/dev/null; then
    :
else
    echo "warning: response body is not valid JSON, see response-body.raw" >&2
fi

if [ "$HTTP_STATUS" -ge 200 ] && [ "$HTTP_STATUS" -lt 300 ] && [ -f "$DIR/response-body.json" ]; then
    COUNT="$(jq '(.data // []) | length' "$DIR/response-body.json")"
    echo "images returned: $COUNT"
    for i in $(seq 0 $((COUNT - 1))); do
        B64="$(jq -r ".data[$i].b64_json // empty" "$DIR/response-body.json")"
        if [ -n "$B64" ]; then
            OUT="$DIR/image-$i.$RESOLVED_FORMAT"
            echo "$B64" | base64 -d >"$OUT"
            echo "wrote $OUT"
        fi
    done
else
    echo "non-2xx or unparsable response, see $DIR for details" >&2
fi

echo "done. inspect $DIR (wire-trace.txt has the full raw request/response)"
