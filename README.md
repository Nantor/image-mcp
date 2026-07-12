# image-mcp

An MCP (Model Context Protocol) server for image generation and editing powered by LiteLLM.

## Features

- **Create** - Generate images from text prompts
- **Edit** - Edit existing images using natural language
- **List Models** - Query available image models from configuration
- **output_path** - Always writes generated images to a filesystem path you specify

## Installation

```bash
cargo install --path .
```

## Configuration

Startup requires a config file at an OS-specific path resolved via the `dirs` crate:

- **Linux**: `~/.config/image-mcp/config.json`
- **macOS**: `~/Library/Application Support/image-mcp/config.json`
- **Windows**: `%APPDATA%\\image-mcp\\config.json`

Example `config.json`:

```jsonc
{
  "lite_llm": {
    "base_url": "http://localhost:4000",
    "api_key": "sk-...",
    "request_timeout_secs": 180
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

Notes:

- The file must exist and be valid JSONC (comments allowed) on startup.
- Missing or invalid config terminates the process at startup with a clear stderr error.

## Editing images

`edit` requires `input_path` — one or more filesystem paths to image
files, read from disk. Multiple entries let the model compose/reference
all of them in a single edit (e.g. combining a subject from one image
with a background from another). A missing or empty `input_path` is a
validation error.

Example tool call:

```jsonc
{ "prompt": "add a hat", "input_path": ["/home/me/Pictures/photo.png"], "output_path": "/home/me/Pictures/photo-with-hat.png" }
```

## Writing images to disk

Both `create` and `edit` require an `output_path` string: an exact
destination file path. Parent directories are created as needed, and the
configured/requested format's extension is appended if the path has none.

If `n > 1` produces multiple images, every image gets a `-1`, `-2`, ...
suffix inserted before the extension (e.g. `out.png` becomes `out-1.png`,
`out-2.png`, ...); with exactly one image, the exact requested path is
used as-is.

Example tool call:

```jsonc
{ "prompt": "a red bicycle", "output_path": "/home/me/Pictures/bicycle.png" }
```

## Building & Testing

```bash
cargo check --all --all-targets --all-features
cargo test
cargo clippy --all --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

## Architecture

| File | Purpose |
|------|---------|
| `src/main.rs` | Initializes tracing, loads config, serves MCP over stdio |
| `src/server.rs` | Wiring hub for `create`, `edit`, and `list_models` tools |
| `src/tools/mod.rs` | Shared contract: `ImageParams`, config defaults, responses |
| `src/litellm.rs` | All LiteLLM HTTP behaviour |

See `PLAN.md` for the complete design spec.
