#!/usr/bin/env bash
# Shared helpers for the create.sh / edit.sh raw-request capture scripts.
#
# These scripts hit LiteLLM directly with plain curl, building the exact
# request shape that src/litellm.rs sends, so you can inspect the raw
# wire-level request/response without going through the MCP server at all.
#
# Kept in sync with:
#   - src/config.rs      (config file location + shape)
#   - src/tools/mod.rs   (ImageParams -> ResolvedParams defaulting)
#   - src/litellm.rs     (exact request bodies for generate()/edit())

set -euo pipefail

CONFIG_PATH="${IMAGE_MCP_CONFIG:-$HOME/.config/image-mcp/config.json}"
CAPTURE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/captures"

die() {
    echo "error: $*" >&2
    exit 1
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "required command '$1' not found on PATH"
}

# Strips // line comments and /* */ block comments from JSONC while
# respecting string literals, then validates + re-emits as compact JSON.
# Mirrors what `jsonc_parser` does for config.rs::load_config, closely
# enough for a config file with the documented shape.
strip_jsonc() {
    python3 - "$1" <<'PY'
import sys, json

path = sys.argv[1]
with open(path, "r") as f:
    src = f.read()

out = []
i = 0
n = len(src)
in_string = False
escape = False
while i < n:
    c = src[i]
    if in_string:
        out.append(c)
        if escape:
            escape = False
        elif c == "\\":
            escape = True
        elif c == '"':
            in_string = False
        i += 1
        continue
    if c == '"':
        in_string = True
        out.append(c)
        i += 1
        continue
    if c == "/" and i + 1 < n and src[i + 1] == "/":
        while i < n and src[i] != "\n":
            i += 1
        continue
    if c == "/" and i + 1 < n and src[i + 1] == "*":
        i += 2
        while i + 1 < n and not (src[i] == "*" and src[i + 1] == "/"):
            i += 1
        i += 2
        continue
    out.append(c)
    i += 1

cleaned = "".join(out)
try:
    data = json.loads(cleaned)
except json.JSONDecodeError as e:
    sys.stderr.write(f"failed to parse {path} as JSONC: {e}\n")
    sys.exit(1)

json.dump(data, sys.stdout)
PY
}

# Loads config.json, exits with a clear error if missing/invalid — same
# behavior as config.rs::load_config (no auto-creation, no defaults).
load_config_json() {
    [ -f "$CONFIG_PATH" ] || die "config file not found at $CONFIG_PATH (see PLAN.md for the expected shape)"
    strip_jsonc "$CONFIG_PATH"
}

cfg() {
    # cfg <config_json> <jq_filter>
    echo "$1" | jq -er "$2"
}

new_capture_dir() {
    # new_capture_dir <label> -> prints the created dir path
    local label="$1"
    local ts
    ts="$(date -u +%Y%m%dT%H%M%SZ)"
    local dir="$CAPTURE_ROOT/$label/$ts"
    mkdir -p "$dir"
    echo "$dir"
}

# Mirrors litellm.rs::sniff_image_type — sniffs magic bytes, defaults to
# png. Prints "<extension> <mime>".
sniff_image_type() {
    local file="$1"
    local hex
    hex="$(od -An -tx1 -N12 "$file" | tr -d ' \n')"

    case "$hex" in
        89504e470d0a1a0a*) echo "png image/png" ;;
        ffd8ff*) echo "jpg image/jpeg" ;;
        52494646????????57454250*) echo "webp image/webp" ;;
        *) echo "png image/png" ;;
    esac
}
