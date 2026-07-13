# AGENTS.md

## Commands

- CI runs, in this order: `cargo check --all --all-targets --all-features`, `cargo test --all`, `cargo clippy --all --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.
- `rust-toolchain.toml` pins Rust `1.97.0` (matching `RUST_VERSION` in `.github/workflows/ci.yml`); `rustup` will auto-install it on first use in this directory.
- Focused test runs use normal Cargo filters, e.g. `cargo test missing_image_parameter_returns_error`.

## Architecture

- Single binary crate: `src/main.rs` initializes `tracing`, loads config, and serves the MCP server over stdio.
- `src/server.rs` is the wiring hub: the only exposed tools are `create`, `edit`, and `list_models`.
- `src/tools/mod.rs` is the shared contract layer: `ImageParams`, config-default resolution, and the shared response path that writes images to disk and returns their filenames live there.
- `src/litellm.rs` owns all HTTP behavior for the upstream OpenAI-compatible image API.

## Gotchas

- Stdout is protocol-only. Logging must stay on stderr; `main.rs` already configures `tracing` that way.
- Startup config is mandatory at `~/.config/image-mcp/config.json`, parsed as JSONC. Missing or invalid config is a process-exit startup failure, not a tool error.
- Runtime failures must surface as MCP tool errors (`CallToolResult::error`), not process exits.
- Per-call tool params override `create_defaults` / `edit_defaults`; do not add built-in fallback defaults.
- `list_models` is local-only. It returns `config.image_models` and must not call the upstream image API.
- `edit` is prompt-driven only: no mask/inpainting API surface.
- `edit` accepts one or more on-disk `input_path` entries and sends each to the upstream image API as its own `image[]` multipart part.
- `create` sends `response_format: "b64_json"`; `edit` intentionally does not. `PLAN.md` documents the verified behavior behind that split.
- Both tools require `output_path` (an exact destination file path) and always write image data to disk — there is no inline MCP `image` content block and no `save`/`save_path` toggle.
- `output_path`'s extension is appended if missing. With `n > 1`, every generated image gets a `-1`, `-2`, ... suffix inserted before the extension (1-based); with exactly one image, the exact requested path is used as-is.

## Manual Tests

- `docs/manual-tests/` contains ad-hoc test session documentation (markdown + PNG output). These are manual, non-automated, and not run by CI.
- `docs/manual-tests/images/` holds the ~30 MB of PNG capture files (19 PNGs across Gemini and GPT model families).

## Repo Notes

- `PLAN.md` is still the design spec; update it when implementation behavior changes.

