# AGENTS

This subfolder contains the Rust sources for the `image-mcp` MCP server.

- Entry point: `main.rs` (sets up tracing, loads config, and serves the MCP server over stdio).
- Server wiring: `server.rs` (exposes the MCP tools: `create`, `edit`, `list_models`, `image_info`, and `image_resize`).
- Shared tool contract: `tools/mod.rs` (owns `ImageParams`, config default resolution, and disk write behavior for image outputs).
- Upstream HTTP client: `image_api.rs` (all OpenAI-compatible image API calls live here).

For global commands, architecture, and gotchas, see the root-level `AGENTS.md`.
