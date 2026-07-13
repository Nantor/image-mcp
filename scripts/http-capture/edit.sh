#!/usr/bin/env bash
# Raw-request capture script for the `edit` tool's image API call.
#
# Mirrors ImageApiClient::edit() in src/image_api.rs, with one deliberate
# EXPERIMENTAL deviation: this script accepts a repeatable --image flag
# and sends one `image[]` multipart part per image, per OpenAI's
# /v1/images/edits spec which accepts an *array* of input images. The
# current Rust implementation (image_api.rs::edit / ImageParams) only
# supports a single `image: Option<String>` — this script exists to
# test whether the live OpenAI-compatible proxy/model honors multi-image edits
# before committing to that change in the production code.
#
#   POST {base_url}/v1/images/edits
#   Content-Type: multipart/form-data
#   Authorization: Bearer <api_key>
#   fields: prompt, model, n, size, output_format
#   file part(s): image[] = <input image>, filename "image.<ext>",
#                 sniffed mime type (same sniff_image_type() logic as
#                 litellm.rs), repeated once per --image flag
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
#   # experimental multi-image edit (repeat --image):
#   ./edit.sh --prompt "combine these" \
#             --image /path/to/a.png --image /path/to/b.png --image /path/to/c.png
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
IMAGE_PATHS=()
MODEL=""
N=""
SIZE=""
FORMAT=""

usage() {
    cat <<EOF
Usage: $0 --prompt "<text>" --image <path> [--image <path> ...] [--model MODEL] [--n N] [--size WxH] [--format png|jpg|webp]

--image may be repeated to send multiple image[] parts (experimental —
see script header comment).
EOF
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prompt) PROMPT="$2"; shift 2 ;;
        --image) IMAGE_PATHS+=("$2"); shift 2 ;;
        --model) MODEL="$2"; shift 2 ;;
        --n) N="$2"; shift 2 ;;
        --size) SIZE="$2"; shift 2 ;;
        --format) FORMAT="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "unknown argument: $1" >&2; usage ;;
    esac
done

[ -n "$PROMPT" ] || { echo "error: --prompt is required" >&2; usage; }
[ "${#IMAGE_PATHS[@]}" -ge 1 ] || { echo "error: at least one --image is required (edit has no mask param — these are the sole input images)" >&2; usage; }
for p in "${IMAGE_PATHS[@]}"; do
    [ -f "$p" ] || die "image file not found: $p"
done

CONFIG_JSON="$(load_config_json)"

BASE_URL="$(cfg "$CONFIG_JSON" '.image_api.base_url' | sed 's:/*$::' | sed 's:/v1/*$::')"
API_KEY="$(cfg "$CONFIG_JSON" '.image_api.api_key')"

# Resolve against edit_defaults, same precedence as ImageParams::resolve.
RESOLVED_MODEL="${MODEL:-$(cfg "$CONFIG_JSON" '.edit_defaults.model')}"
RESOLVED_N="${N:-$(cfg "$CONFIG_JSON" '.edit_defaults.n')}"
RESOLVED_SIZE="${SIZE:-$(cfg "$CONFIG_JSON" '.edit_defaults.size')}"
RESOLVED_FORMAT="${FORMAT:-$(cfg "$CONFIG_JSON" '.edit_defaults.format')}"

URL="$BASE_URL/v1/images/edits"

DIR="$(new_capture_dir edit)"
echo "capturing to: $DIR"

# Same magic-byte sniff as litellm.rs::sniff_image_type, used to name/type
# each image[] part the same way the Rust client does. Sniffed once per
# input image since a multi-image edit may mix formats.
CURL_IMAGE_ARGS=()
INPUT_IMAGES_JSON="[]"
for idx in "${!IMAGE_PATHS[@]}"; do
    p="${IMAGE_PATHS[$idx]}"
    read -r ext mime <<<"$(sniff_image_type "$p")"
    cp "$p" "$DIR/input-image-$idx.$ext"
    CURL_IMAGE_ARGS+=(-F "image[]=@${p};filename=image-$idx.$ext;type=$mime")
    INPUT_IMAGES_JSON="$(echo "$INPUT_IMAGES_JSON" | jq \
        --arg src "$p" --arg ext "$ext" --arg mime "$mime" --arg fn "image-$idx.$ext" \
        '. + [{source_path: $src, sniffed_extension: $ext, sniffed_mime: $mime, sent_as_filename: $fn}]')"
done

jq -n \
    --arg endpoint "$URL" \
    --arg prompt "$PROMPT" \
    --arg model "$RESOLVED_MODEL" \
    --argjson n "$RESOLVED_N" \
    --arg size "$RESOLVED_SIZE" \
    --arg format "$RESOLVED_FORMAT" \
    --argjson input_images "$INPUT_IMAGES_JSON" \
    '{
        endpoint: $endpoint,
        method: "POST",
        resolved_params: {prompt: $prompt, model: $model, n: $n, size: $size, format: $format},
        input_images: $input_images
    }' >"$DIR/meta.json"

# --trace-ascii captures the literal bytes sent/received on the wire
# (headers + multipart body, both directions) for later low-level
# analysis. Field order below matches Form::new() in litellm.rs, with
# one image[] part appended per --image flag (experimental).
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
    "${CURL_IMAGE_ARGS[@]}")"

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
