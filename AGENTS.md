# AGENTS.md

## Status

Pre-implementation. The repo currently contains only `PLAN.md` — no
`Cargo.toml`, `src/`, or build/test tooling exists yet. There are no dev
commands to run yet. Treat `PLAN.md` as the design spec and keep it in sync
with any implementation decisions that diverge from it.

## Project

MCP server (Rust) exposing AI image generation/editing tools, backed by a
LiteLLM proxy (OpenAI-compatible image API). Tools: `create`, `edit`,
`list_models`. Full spec, schemas, and config format are in `PLAN.md` —
read it before implementing.

## Hard constraints from the plan (easy to get wrong)

- **Transport is stdio; stdout is reserved for JSON-RPC.** All logging
  (via `tracing`) must go to stderr — never write to stdout outside the
  protocol layer.
- **Config at `~/.config/image-mcp/config.json` (JSONC) must exist on
  startup.** No auto-creation, no built-in defaults, no merging. Missing/
  invalid config → process exits immediately with a clear stderr message
  (this is a startup failure, not a tool error).
- **Runtime failures are tool errors, not process exits** — LiteLLM
  unreachable, bad model, invalid image, disk write failure all become an
  MCP tool result with `isError: true` so the calling LLM can retry/adjust.
- `edit` has no mask parameter — masking/inpainting, image-to-image
  strength, and outpaint/crop/resize are explicitly out of scope; editing
  is purely prompt-driven (relying on models like `gpt-image-1`).
- `create` and `edit` share one params struct but hit different endpoints
  with different encodings: `create` → JSON POST to
  `/v1/images/generations`; `edit` → multipart/form-data POST to
  `/v1/images/edits` (image as `image[]` file part).
- Both requests set `response_format: b64_json` — this is unverified
  against real models; check LiteLLM/model behavior before assuming it
  works.
- `list_models` never calls LiteLLM — it just returns `image_models` from
  local config.
- Per-call tool params override config defaults (`create_defaults` /
  `edit_defaults`), not the other way around.
- `save: true` writes the image to disk and returns a `text` path; default
  (`save: false`) returns an inline MCP `image` content block. Large
  `n`/`size` combos with inline images may hit stdio message-size limits.
</content>
</invoke>
