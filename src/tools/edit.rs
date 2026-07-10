use base64::Engine as _;
use rmcp::model::{CallToolResult, ContentBlock};

use crate::config::Config;
use crate::litellm::LiteLlmClient;

use super::ImageParams;

/// Runs the `edit` (prompt-driven image editing) tool: resolves params
/// against `edit_defaults`, decodes the required input `image`, calls
/// LiteLLM's `/v1/images/edits`, and returns either an inline image block or
/// a saved file path per `save`.
pub async fn run(config: &Config, client: &LiteLlmClient, params: ImageParams) -> CallToolResult {
    let Some(image_b64) = params.image.clone() else {
        return CallToolResult::error(vec![ContentBlock::text(
            "edit requires an `image` parameter (base64-encoded input image)",
        )]);
    };

    let image_bytes = match base64::engine::general_purpose::STANDARD.decode(&image_b64) {
        Ok(bytes) => bytes,
        Err(err) => {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "invalid base64 in `image` parameter: {err}"
            ))]);
        }
    };

    let resolved = params.resolve(&config.edit_defaults);

    let images = match client.edit(&resolved, image_bytes).await {
        Ok(images) => images,
        Err(err) => {
            return CallToolResult::error(vec![ContentBlock::text(format!("edit failed: {err}"))]);
        }
    };

    super::respond_with_images(images, resolved.format, resolved.save)
}
