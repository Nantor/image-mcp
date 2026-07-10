use rmcp::model::{CallToolResult, ContentBlock};

use crate::config::Config;

/// Runs the `list_models` tool. Never calls LiteLLM — just returns the
/// configured `image_models` list.
pub fn run(config: &Config) -> CallToolResult {
    let json = serde_json::json!({ "image_models": config.image_models });
    CallToolResult::success(vec![ContentBlock::text(json.to_string())])
}
