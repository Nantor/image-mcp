use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};

use crate::config::Config;
use crate::litellm::LiteLlmClient;
use crate::tools::{ImageParams, create, edit, list_models};

#[derive(Clone)]
pub struct ImageMcpServer {
    config: std::sync::Arc<Config>,
    client: std::sync::Arc<LiteLlmClient>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ImageMcpServer {
    pub fn new(config: Config) -> Self {
        let client = LiteLlmClient::new(&config.lite_llm);
        Self {
            config: std::sync::Arc::new(config),
            client: std::sync::Arc::new(client),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Generate an image from a text prompt (text-to-image).")]
    async fn create(
        &self,
        Parameters(params): Parameters<ImageParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(create::run(&self.config, &self.client, params).await)
    }

    #[tool(
        description = "Edit one or more images using a natural-language prompt. Requires at least one base64-encoded input `image`; when multiple are given, the model can compose/reference all of them (e.g. combining a subject from one image with a background from another). There is no mask/inpainting support — describe the desired edit in `prompt`."
    )]
    async fn edit(
        &self,
        Parameters(params): Parameters<ImageParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(edit::run(&self.config, &self.client, params).await)
    }

    #[tool(description = "List the configured image-capable models.")]
    fn list_models(&self) -> Result<CallToolResult, McpError> {
        Ok(list_models::run(&self.config))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ImageMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Image generation and editing backed by a LiteLLM proxy. Tools: create (text-to-image), edit (prompt-driven image editing, no mask support), list_models (configured image models).".to_string(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Format, ImageDefaults, LiteLlmConfig};
    use rmcp::model::ContentBlock;

    fn sample_config() -> Config {
        Config {
            lite_llm: LiteLlmConfig {
                base_url: "http://localhost:4000".to_string(),
                api_key: "test-key".to_string(),
                request_timeout_secs: None,
            },
            image_models: vec!["gpt-image-1".to_string()],
            create_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Png,
                save: false,
            },
            edit_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
                save: false,
            },
            payload_limits: crate::config::PayloadLimits {
                warn_inline_bytes: crate::config::DEFAULT_WARN_INLINE_BYTES,
                max_inline_bytes: crate::config::DEFAULT_MAX_INLINE_BYTES,
            },
        }
    }

    #[test]
    fn new_stores_config_and_builds_client() {
        let server = ImageMcpServer::new(sample_config());
        assert_eq!(server.config.image_models, vec!["gpt-image-1".to_string()]);
    }

    #[test]
    fn get_info_advertises_tools_and_instructions() {
        let server = ImageMcpServer::new(sample_config());
        let info = server.get_info();

        assert!(info.capabilities.tools.is_some());
        assert_eq!(info.protocol_version, ProtocolVersion::V_2024_11_05);
        let instructions = info.instructions.expect("instructions should be set");
        assert!(instructions.contains("create"));
        assert!(instructions.contains("edit"));
        assert!(instructions.contains("list_models"));
    }

    #[test]
    fn list_models_tool_reflects_config() {
        let server = ImageMcpServer::new(sample_config());
        let result = server.list_models().expect("list_models should not error");

        assert_eq!(result.is_error, Some(false));
        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("gpt-image-1"));
    }

    #[tokio::test]
    async fn create_tool_surfaces_validation_error() {
        let server = ImageMcpServer::new(sample_config());
        let params = Parameters(ImageParams {
            prompt: "   ".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image: None,
            save: None,
            save_path: None,
        });

        let result = server
            .create(params)
            .await
            .expect("create should return a CallToolResult, not an McpError");
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn edit_tool_surfaces_missing_image_error() {
        let server = ImageMcpServer::new(sample_config());
        let params = Parameters(ImageParams {
            prompt: "edit this".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image: None,
            save: None,
            save_path: None,
        });

        let result = server
            .edit(params)
            .await
            .expect("edit should return a CallToolResult, not an McpError");
        assert_eq!(result.is_error, Some(true));
    }
}
