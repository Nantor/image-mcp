use rmcp::model::{CallToolResult, ContentBlock};

use crate::config::Config;
use crate::litellm::LiteLlmClient;

use super::ImageParams;

/// Runs the `create` (text-to-image) tool: resolves params against
/// `create_defaults`, calls LiteLLM's `/v1/images/generations`, and returns
/// either an inline image block or a saved file path per `save`.
pub async fn run(config: &Config, client: &LiteLlmClient, params: ImageParams) -> CallToolResult {
    let resolved = params.resolve(&config.create_defaults);

    if let Err(err) = resolved.validate() {
        return CallToolResult::error(vec![ContentBlock::text(err)]);
    }

    let images = match client.generate(&resolved).await {
        Ok(images) => images,
        Err(err) => {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "create failed: {err}"
            ))]);
        }
    };

    super::respond_with_images(images, resolved.format, resolved.save)
}
