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
    input_path: Option<Vec<String>>, // on-disk input image path(s), read and base64-encoded internally — required for `edit` (>=1), unused for `create`
    output_path: String,     // required filesystem path to write the output image(s) to
}
```

- `edit` requires `input_path` (at least one entry); a missing param, or
  an empty list, is a validation error surfaced before any network call.
  `input_path` entries are read from disk with `std::fs::read`; a missing
  file, unreadable file, or empty file is likewise a validation error.
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

Both tools always write their output image(s) to disk and return the
written filename(s) as `text` content blocks — there is no inline
base64 image content block.

`output_path` is a required, exact destination file path: parent
directories are created as needed, and the resolved `format`'s extension
is appended if the path has none.

- With `n == 1`, the exact requested path is used as-is.
- With `n > 1`, every image gets a `-<index>` suffix (1-based) inserted
  before the extension (e.g. `out.png` becomes `out-1.png`, `out-2.png`,
  ...), so a multi-image response never silently overwrites itself and
  filenames are predictable.

## Config

Location is OS-specific and resolved via the `dirs` crate. Typical paths:

- **Linux**: `~/.config/image-mcp/config.json`
- **macOS**: `~/Library/Application Support/image-mcp/config.json`
- **Windows**: `%APPDATA%\\image-mcp\\config.json`

The config file is JSONC (comments allowed).
**It must exist on startup or the process exits with a clear stderr error.**
There are no hardcoded defaults, no auto-creation, and no merging with built-in defaults.

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
    "format": "png"
  },
  "edit_defaults": {
    "model": "gpt-image-1",
    "n": 1,
    "size": "1024x1024",
    "format": "png"
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
│   └── image_store.rs    # writes decoded images to the exact `output_path`, returns path
```

## Release flow

- `CI` runs on pushes and pull requests targeting `master`, using Rust `1.85.0` for `cargo check`, `cargo test`, `cargo clippy`, and `cargo fmt --check`.
- `Release` has two entry points:
  - `workflow_run` after a successful `CI` run on `master`
  - `push` for tags matching `v*`
- The `workflow_run` path is tag-creation only:
  - read the crate version from `Cargo.toml`
  - create and push tag `v<version>` for the validated commit if that tag does not already exist
- The tag-push path performs the actual release work:
  - create the GitHub Release if missing
  - build release artifacts for all configured targets (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`)
  - upload artifacts to the GitHub Release
- Release uploads are intended to be rerunnable:
  - existing tags/releases are treated as no-ops
  - asset uploads use `gh release upload --clobber`
- The Linux `aarch64-unknown-linux-gnu` release build requires `gcc-aarch64-linux-gnu` as the linker on GitHub-hosted Ubuntu runners.

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
- stdio message-size limits are no longer a concern for tool responses:
  both tools always write image data to disk and return only the
  filename(s) as small text content blocks, never inline base64 image
  data.
