# image-mcp — Plan

An MCP server, written in Rust, exposing AI image generation and editing tools
backed by a LiteLLM proxy (OpenAI-compatible image API).

## Scope

- **create** — text-to-image generation
- **edit** — prompt-driven image editing (no mask param; relies on
  prompt-native edit models like `gpt-image-1`)
- **list_models** — returns the configured list of image-capable models

Mask-based inpainting, image-to-image style transfer via reference image
strength, and outpaint/crop/resize as distinct operations are explicitly
**out of scope** — all covered by natural-language instructions inside
`prompt` for `edit`, or dropped as unsupported.

## Tool schemas

Both tools share one params struct:

```rust
struct ImageParams {
    prompt: String,
    model: Option<String>,   // falls back to config default per mode
    n: Option<u32>,          // falls back to config default per mode
    size: Option<String>,    // e.g. "1024x1024", falls back to config default
    format: Option<Format>,  // png | jpg | webp
    image: Option<Vec<String>>, // base64 input image(s) — required for `edit` (>=1), unused for `create`
    save: Option<bool>,      // true = write to disk & return path, false = inline image content
}
```

- `create` → `POST {base_url}/v1/images/generations` (JSON body)
- `edit` → `POST {base_url}/v1/images/edits` (multipart/form-data: one `image[]` file part per input image, `prompt`/`model` as text fields — per LiteLLM's OpenAPI spec; verified against a live LiteLLM proxy that multiple `image[]` parts in one request let the model compose/reference all of them, e.g. combining a subject from one image with a background from another)
- `create` explicitly sets `response_format: b64_json` and this is
  honored by the models tested against.
- `edit` does **not** send `response_format` — verified against a live
  LiteLLM proxy that at least `gpt-image-1.5` rejects it on this endpoint
  with a 400 (`Unknown parameter: 'response_format'`), while
  `/v1/images/generations` accepts it fine. The edits endpoint returns
  `b64_json` data by default regardless.
- `list_models` takes no input, returns `image_models` straight from config
  (no LiteLLM call).

## Response shape

- `save: false` (default) → native MCP `image` content block
  (`{ type: "image", data: <base64>, mimeType: "image/png" }`)
- `save: true` → write to disk, return `text` content block with the file path

## Config

Location: `~/.config/image-mcp/config.json` (JSONC — comments allowed).
**Must exist on startup or the process exits with a clear stderr error.**
No hardcoded defaults, no auto-creation, no merging with built-in defaults.

```jsonc
{
  "lite_llm": {
    "base_url": "http://localhost:4000",
    "api_key": "sk-...",
    "request_timeout_secs": 180 // optional, defaults to 180
  },
  "image_models": ["gpt-image-1"],
  "create_defaults": {
    "model": "gpt-image-1",
    "n": 1,
    "size": "1024x1024",
    "format": "png",
    "save": false
  },
  "edit_defaults": {
    "model": "gpt-image-1",
    "n": 1,
    "size": "1024x1024",
    "format": "png",
    "save": false
  }
}
```

Per-call params in a tool invocation override the matching config default.

## Stack

- **Language**: Rust
- **MCP SDK**: [`rmcp`](https://docs.rs/rmcp) (official SDK) — features:
  `server`, `macros`, `transport-io` (stdio), `schemars` (for
  `JsonSchema`-derived tool param schemas)
- **HTTP client**: `reqwest` — direct calls to LiteLLM, no SDK wrapper
- **Transport**: stdio
- **Logging**: `tracing` to stderr (stdout is reserved for JSON-RPC on
  stdio transport — never log there)

## Error handling

- Runtime failures (LiteLLM unreachable, bad model, invalid image, disk
  write failure) → MCP tool result with `isError: true`, so the calling LLM
  can see the error and retry/adjust.
- Config missing or invalid at startup → process exits immediately with a
  clear stderr message. Not a runtime tool error.

## Project structure

```text
image-mcp/
├── Cargo.toml
├── src/
│   ├── main.rs          # entrypoint: load config, init tracing, serve stdio
│   ├── config.rs        # JSONC config loading + structs
│   ├── server.rs        # ImageMcpServer: rmcp tool_router wiring for create/edit/list_models
│   ├── litellm.rs        # reqwest client: generate() [JSON], edit() [multipart], shared b64_json response parsing
│   ├── tools/
│   │   ├── mod.rs        # ImageParams/ResolvedParams, validation, shared respond_with_images
│   │   ├── create.rs     # `create` tool impl
│   │   ├── edit.rs       # `edit` tool impl
│   │   └── list_models.rs
│   └── image_store.rs    # handles `save: true` — writes to disk, returns path
```

## Validated behavior

All items originally flagged for validation during implementation have
been resolved and are covered by tests:

- `response_format: b64_json` — honored by `create` on
  `/v1/images/generations`; rejected by `edit` on `/v1/images/edits` (at
  least `gpt-image-1.5` returns a 400 `Unknown parameter`), so `edit` no
  longer sends it. See "Tool schemas" above.
- Multipart field names for `/v1/images/edits` — confirmed against a live
  LiteLLM instance: plain form fields for
  `prompt`/`model`/`n`/`size`/`output_format`, plus one `image[]` file
  part per input image (multiple parts accepted and composited by the
  model).
- stdio message-size limits — the `rmcp` stdio transport itself imposes no
  cap (`JsonRpcMessageCodec::default()` uses `max_length: usize::MAX`, and
  `receive()` reads into an unbounded, growable buffer). Any practical
  limit comes from the MCP client/host, which varies and can't be
  verified from this repo. Real captures under
  `scripts/http-capture/captures/` show a single 1024x1024 PNG already
  runs ~2-3 MB base64-encoded, scaling quickly with `n`. As a mitigation,
  `respond_with_images` logs a stderr warning (not an error) when the
  total inline payload exceeds 4 MB, suggesting `save: true` as an
  alternative.
