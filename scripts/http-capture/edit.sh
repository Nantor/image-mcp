#!/usr/bin/env bash
# Raw-request capture script for the `edit` tool's LiteLLM call.
#
# Mirrors LiteLlmClient::edit() in src/litellm.rs exactly:
#   POST {base_url}/v1/images/edits
#   Content-Type: multipart/form-data
#   Authorization: Bearer <api_key>
#   fields: prompt, model, n, size, output_format
#   file part: image[] = <input image>, filename "image.<ext>", sniffed
#              mime type (same sniff_image_type() logic as litellm.rs)
#
# Note: unlike create.sh, this does NOT send response_format. At least
# gpt-image-1.5 rejects it on /v1/images/edits with a 400 "Unknown
# parameter: 'response_format'", even though /v1/images/generations
# accepts it fine. The endpoint returns b64_json by default anyway.
#
# Params resolve the same way ImageParams::resolve() does in
# src/tools/mod.rs: CLI flags override config's `edit_defaults`, unmet
# flags fall back to the config default for each field.
#
# Every raw byte sent/received on the wire is captured via curl's
# --trace-ascii into the capture directory, alongside a resolved-params
# summary and the parsed image outputs (if any).
#
# Usage:
#   ./edit.sh --prompt "add a hat" --image /path/to/input.png \
#             [--model gpt-image-1] [--n 1] [--size 1024x1024] [--format png]
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
IMAGE_PATH=""
MODEL=""
N=""
SIZE=""
FORMAT=""

usage() {
    cat <<EOF
Usage: $0 --prompt "<text>" --image <path> [--model MODEL] [--n N] [--size WxH] [--format png|jpg|webp]
EOF
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prompt) PROMPT="$2"; shift 2 ;;
        --image) IMAGE_PATH="$2"; shift 2 ;;
        --model) MODEL="$2"; shift 2 ;;
        --n) N="$2"; shift 2 ;;
        --size) SIZE="$2"; shift 2 ;;
        --format) FORMAT="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "unknown argument: $1" >&2; usage ;;
    esac
done

[ -n "$PROMPT" ] || { echo "error: --prompt is required" >&2; usage; }
[ -n "$IMAGE_PATH" ] || { echo "error: --image is required (edit has no mask param — this is the sole input image)" >&2; usage; }
[ -f "$IMAGE_PATH" ] || die "image file not found: $IMAGE_PATH"

CONFIG_JSON="$(load_config_json)"

BASE_URL="$(cfg "$CONFIG_JSON" '.lite_llm.base_url' | sed 's:/*$::' | sed 's:/v1/*$::')"
API_KEY="$(cfg "$CONFIG_JSON" '.lite_llm.api_key')"

# Resolve against edit_defaults, same precedence as ImageParams::resolve.
RESOLVED_MODEL="${MODEL:-$(cfg "$CONFIG_JSON" '.edit_defaults.model')}"
RESOLVED_N="${N:-$(cfg "$CONFIG_JSON" '.edit_defaults.n')}"
RESOLVED_SIZE="${SIZE:-$(cfg "$CONFIG_JSON" '.edit_defaults.size')}"
RESOLVED_FORMAT="${FORMAT:-$(cfg "$CONFIG_JSON" '.edit_defaults.format')}"

URL="$BASE_URL/v1/images/edits"

# Same magic-byte sniff as litellm.rs::sniff_image_type, used to name/type
# the image[] part the same way the Rust client does.
read -r SNIFF_EXT SNIFF_MIME <<<"$(sniff_image_type "$IMAGE_PATH")"

DIR="$(new_capture_dir edit)"
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
  },
  "input_image": {
    "source_path": "$IMAGE_PATH",
    "sniffed_extension": "$SNIFF_EXT",
    "sniffed_mime": "$SNIFF_MIME",
    "sent_as_filename": "image.$SNIFF_EXT"
  }
}
EOF

cp "$IMAGE_PATH" "$DIR/input-image.$SNIFF_EXT"

# --trace-ascii captures the literal bytes sent/received on the wire
# (headers + multipart body, both directions) for later low-level
# analysis. Field order below matches Form::new() in litellm.rs.
HTTP_STATUS="$(curl -sS \
    --trace-ascii "$DIR/wire-trace.txt" \
    -o "$DIR/response-body.raw" \
    -D "$DIR/response-headers.txt" \
    -w '%{http_code}' \
    -X POST "$URL" \
    -H "Authorization: Bearer $API_KEY" \
    -F "prompt=$PROMPT" \
    -F "model=$RESOLVED_MODEL" \
    -F "n=$RESOLVED_N" \
    -F "size=$RESOLVED_SIZE" \
    -F "output_format=$RESOLVED_FORMAT" \
    -F "image[]=@${IMAGE_PATH};filename=image.$SNIFF_EXT;type=$SNIFF_MIME")"

echo "$HTTP_STATUS" >"$DIR/response-status.txt"
echo "HTTP status: $HTTP_STATUS"

# Redact the bearer token from the trace copy kept for review.
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
