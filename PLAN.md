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
    image: Option<String>,   // base64 input image — required for `edit`, unused for `create`
    save: Option<bool>,      // true = write to disk & return path, false = inline image content
}
```

- `create` → `POST {base_url}/v1/images/generations` (JSON body)
- `edit` → `POST {base_url}/v1/images/edits` (multipart/form-data: `image[]` file part, `prompt`/`model` as text fields — per LiteLLM's OpenAPI spec)
- Both requests explicitly set `response_format: b64_json` — verify at
  runtime that the target model actually honors this.
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
    "api_key": "sk-..."
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
  `server`, `macros`, `transport-io` (stdio)
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

```
image-mcp/
├── Cargo.toml
├── src/
│   ├── main.rs          # entrypoint: load config, init tracing, serve stdio
│   ├── config.rs        # JSONC config loading + structs
│   ├── litellm.rs        # reqwest client: generate() [JSON], edit() [multipart], shared b64_json response parsing
│   ├── tools/
│   │   ├── mod.rs
│   │   ├── create.rs     # `create` tool impl
│   │   ├── edit.rs       # `edit` tool impl
│   │   └── list_models.rs
│   └── image_store.rs    # handles `save: true` — writes to disk, returns path
```

## Open items to validate during implementation

- Confirm whether target models actually honor `response_format: b64_json`
  on both `/v1/images/generations` and `/v1/images/edits`.
- Confirm exact multipart field names LiteLLM expects for `/v1/images/edits`
  beyond `image[]` (e.g. does `prompt` need to be a plain form field or
  JSON-encoded?) — validate with a live LiteLLM instance before finalizing
  `litellm.rs`.
- Verify practical message-size limits for the `image` content block over
  stdio with your actual MCP client, since large `n`/`size` combinations may
  need `save: true` to avoid oversized JSON-RPC messages.
