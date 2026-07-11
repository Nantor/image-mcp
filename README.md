# image-mcp

An MCP (Model Context Protocol) server for image generation and editing powered by LiteLLM.

## Features

- **Create** - Generate images from text prompts
- **Edit** - Edit existing images using natural language
- **List Models** - Query available image models from configuration

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

Notes:

- The file must exist and be valid JSONC (comments allowed) on startup.
- Missing or invalid config terminates the process at startup with a clear stderr error.

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
