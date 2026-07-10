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
