# image-mcp

An MCP (Model Context Protocol) server for image generation and editing backed by an OpenAI-compatible image API (or proxy).

The upstream service can be configured to talk to different providers (OpenAI, Azure, etc.), so this server is **provider-agnostic** as long as the configured base URL speaks the OpenAI-style `/v1/images/generations` and `/v1/images/edits` endpoints.

The goal of this server is to be predictable and file-system friendly:

- Every tool call writes images to disk.
- Validation happens before network calls where possible.
- Configuration is explicit and validated at startup.

Most of these behaviors are enforced by tests in `src/`, which are referenced below.

## Features

- **Create** – Generate images from text prompts using the `/v1/images/generations` endpoint of the configured image API.
- **Edit** – Edit existing images using natural language, via the `/v1/images/edits` endpoint.
- **List Models** – Return the configured image models from your local config (no external image API call).
- **Filesystem output** – Always writes generated images to a filesystem path you specify (`output_path`).

## Installation

```bash
cargo install --path .
```

## Configuration

On startup the server loads a single JSONC config file from an OS-specific path resolved via the `dirs` crate:

- **Linux**: `~/.config/image-mcp/config.json`
- **macOS**: `~/Library/Application Support/image-mcp/config.json`
- **Windows**: `%APPDATA%\\image-mcp\\config.json`

Example `config.json`:

```jsonc
{
  "image_api": {
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
- Missing or invalid config terminates the process at startup with a clear stderr error rather than a tool error.
- Defaults are not "magicked" in at runtime: `create_defaults` / `edit_defaults` must be complete and valid. This is tested in `config.rs` (see `validate_config_*` tests).

The `config` module has tests that describe the exact rules:

- `validate_config_rejects_empty_image_models` – `image_models` cannot be empty.
- `validate_config_rejects_defaults_with_unknown_model` – default model names must appear in `image_models`.
- `validate_config_rejects_zero_n` – `n` must be at least `1`.
- `validate_config_rejects_malformed_size` – sizes must look like `WIDTHxHEIGHT` (e.g. `"1024x1024"`).

## Tools & Parameters

All tools are exposed via MCP (see `src/server.rs`). They share a common parameter shape defined by `ImageParams` in `src/tools/mod.rs`.

Common fields:

- `prompt` (string, required) – human-language description of the desired image or edit.
- `model` (string, optional) – overrides the default model for the tool.
- `n` (integer, optional) – number of images to generate.
- `size` (string, optional) – image dimensions like `"1024x1024"`.
- `format` (string, optional) – one of `"png"`, `"jpg"`, or `"webp"`.
- `output_path` (string, required) – where to write the resulting image(s).
- `input_path` (string[]; edit only) – one or more on-disk paths to input images.

Neither `input_path` nor `output_path` enforces an allow-listed root or
sandbox — they accept any filesystem path. Symlinks and `..` traversal are
rejected. This means an untrusted or compromised calling LLM can read any
file readable by the process (via `input_path`) or overwrite any writable
file (via `output_path`). This is an accepted risk under the assumption
that the calling LLM is trusted. See `PLAN.md` for details.

The `ImageParams::resolve` tests (`resolve_all_defaults`, `resolve_all_overrides`, `resolve_partial_override`) in `src/tools/mod.rs` document how these optional fields merge with the configured defaults.

### `create`: text-to-image

Generates images from text using the `/v1/images/generations` endpoint of the configured image API.

Behavior highlights:

- Uses `create_defaults` from config when `model`, `n`, `size`, or `format` are omitted.
- Sends `response_format: "b64_json"` and decodes the first (or `n`) images returned.
- Validates `prompt`, `n`, `size`, and `output_path` before calling the image API.

Example MCP tool parameters:

```jsonc
{
  "prompt": "a red bicycle parked under a street lamp at night",
  "output_path": "/home/me/Pictures/bicycle.png"
}
```

Validation behavior for `create` is covered by `create_tool_surfaces_validation_error` in `src/server.rs`.

### `edit`: prompt-driven image editing

Edits one or more existing images using the `/v1/images/edits` endpoint of the configured image API.

Special rules:

- `input_path` is required and must contain at least one path.
- Each input image is read from disk and sent as its own `image[]` multipart field.
- There is no mask/inpainting surface; describe the desired change in `prompt`.

Example MCP tool parameters:

```jsonc
{
  "prompt": "add a party hat and confetti",
  "input_path": ["/home/me/Pictures/photo.png"],
  "output_path": "/home/me/Pictures/photo-with-party-hat.png"
}
```

If `input_path` is missing for `edit`, the tool returns an MCP error, as covered by `edit_tool_surfaces_missing_image_error` in `src/server.rs`.

### `list_models`: local config only

Returns the configured set of image models from your config file. This never calls the upstream image API.

The `list_models` behavior is exercised by `returns_image_models_field` and `returns_empty_models_list` in `src/tools/list_models.rs`.

## Writing Images to Disk

Both `create` and `edit` require an `output_path` string: an exact destination file path. Parent directories are created as needed, and the configured/requested format's extension is appended if the path has none.

If `n > 1` produces multiple images, every image gets a `-1`, `-2`, ... suffix inserted before the extension (for example, `out.png` becomes `out-1.png`, `out-2.png`, ...). With exactly one image, the exact requested path is used as-is.

Example MCP tool parameters:

```jsonc
{ "prompt": "a red bicycle", "output_path": "/home/me/Pictures/bicycle.png" }
```

This behavior is implemented and tested in `src/tools/mod.rs`:

- `respond_with_images_writes_and_returns_text_paths` – confirms that the paths returned in the MCP response match the files written.
- `respond_with_images_multiple_images_get_numbered_suffixes` – confirms the `-1`, `-2`, ... suffix behavior.
- `suffixed_path_inserts_before_extension` / `suffixed_path_handles_missing_extension` – document the filename logic for multiple images.

The low-level disk writing rules are covered in `src/image_store.rs`:

- `save_image_accepts_valid_png_bytes` – valid images are written as-is.
- `save_image_with_file_target_writes_exact_path_and_keeps_extension` – when you include an extension in `output_path`, that exact path is used.
- `save_image_with_file_target_missing_extension_appends_format_extension` – when you omit an extension, the format's extension is appended.
- `save_image_rejects_invalid_base64` and `save_image_writes_decoded_bytes_with_correct_extension` – obviously wrong data is rejected instead of written to disk.

## Behavior Guarantees (Backed by Tests)

The test suite documents the contract this server aims to provide. Some notable guarantees:

- **Config validation** – `validate_config_*` tests in `src/config.rs` cover `image_models`, default models, `n`, and `size`.
- **Parameter validation** – `validate_*` tests in `src/tools/mod.rs` enforce non-empty `prompt`, `n >= 1`, valid `size` shape, and non-empty `output_path`.
- **MCP surface** – tests in `src/server.rs` (`get_info_advertises_tools_and_instructions`, `create_tool_surfaces_validation_error`, `edit_tool_surfaces_missing_image_error`) ensure the advertised tools and their error behavior match what MCP clients see.
- **HTTP behavior** – unit and integration tests in `src/litellm.rs` (`generate_returns_decoded_images_on_success`, error-path tests, and `normalize_base_url` / `sniff_image_type` tests) exercise the HTTP contract with the upstream image API.

If you are unsure how a particular edge case behaves, searching for the corresponding test is a good starting point.

## Manual Tests

The `docs/manual-tests/` directory contains ad-hoc test session documentation — markdown write-ups with generated PNG captures (Gemini and GPT model families, ~30 MB total across 19 PNGs). These are manual, non-automated records of exploratory sessions and are not run by CI.

## Building & Testing

```bash
cargo check --all --all-targets --all-features
cargo test --all
cargo clippy --all --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

## Architecture

| File | Purpose |
|------|---------|
| `src/main.rs` | Initializes tracing, loads config, validates it, and serves MCP over stdio |
| `src/server.rs` | Wiring hub for `create`, `edit`, and `list_models` tools, including MCP metadata/tests |
| `src/tools/mod.rs` | Shared contract: `ImageParams`, config-default resolution, validation, and image response handling |
| `src/tools/create.rs` | Implementation of the `create` tool, calling the upstream image API's `/v1/images/generations` endpoint and persisting images |
| `src/tools/edit.rs` | Implementation of the `edit` tool, handling input images and calling the upstream image API's `/v1/images/edits` endpoint |
| `src/tools/list_models.rs` | Implementation of `list_models`, returning configured models only |
| `src/image_store.rs` | Base64 decoding, lightweight format checking, and filesystem writes for images |
| `src/litellm.rs` | HTTP client for the upstream OpenAI-compatible image API: base URL normalization, request construction, and response parsing |

See `PLAN.md` for the complete design spec.
