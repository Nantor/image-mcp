use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};

use crate::config::Config;
use crate::image_api::ImageApiClient;
use crate::tools::image_info::ImageInfoParams;
use crate::tools::image_resize::ImageResizeParams;
use crate::tools::{ImageParams, create, edit, image_info, image_resize, list_models};

#[derive(Clone)]
pub struct ImageMcpServer {
    config: std::sync::Arc<Config>,
    client: std::sync::Arc<ImageApiClient>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ImageMcpServer {
    pub fn new(config: Config) -> Self {
        let client = ImageApiClient::new(&config.image_api);
        Self {
            config: std::sync::Arc::new(config),
            client: std::sync::Arc::new(client),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Generate an image from a text prompt. Required: `prompt` (describe what to generate), `output_path` (save path with image extension like .png). Optional: `model`, `n` (number of images), `size` (WIDTHxHEIGHT, e.g. \"1024x1024\"), `format` (png/jpg/webp). Falls back to config defaults."
    )]
    async fn create(
        &self,
        Parameters(params): Parameters<ImageParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(create::run(&self.config, &self.client, params).await)
    }

    #[tool(
        description = "Edit existing images using a text prompt. Required: `prompt` (describe the edit), `input_path` (path(s) to image file(s) to edit), `output_path` (save path with image extension). Optional: `model`, `n` (number of images), `size` (WIDTHxHEIGHT, e.g. \"1024x1024\"), `format` (png/jpg/webp). Falls back to config defaults. Accepts multiple input images. No mask/inpainting — describe the entire edit in the prompt."
    )]
    async fn edit(
        &self,
        Parameters(params): Parameters<ImageParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(edit::run(&self.config, &self.client, params).await)
    }

    #[tool(
        description = "List available image generation models. Returns an `image_models` array of names. Use these names as the `model` parameter in create or edit calls. No params needed."
    )]
    fn list_models(&self) -> Result<CallToolResult, McpError> {
        Ok(list_models::run(&self.config))
    }

    #[tool(
        description = "Inspect an existing image file. Required: `input_path`. Returns format (png/jpg/webp), width, height, and file size in bytes."
    )]
    fn image_info(
        &self,
        Parameters(params): Parameters<ImageInfoParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(image_info::run(params))
    }

    #[tool(
        description = "Resize an image to exact WIDTHxHEIGHT dimensions (stretches). Required: `input_path`, `size` (WIDTHxHEIGHT, e.g. \"1024x1024\"), `output_path` (save path). Optional: `format` (png/jpg/webp; defaults to input format)."
    )]
    fn image_resize(
        &self,
        Parameters(params): Parameters<ImageResizeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(image_resize::run(params))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ImageMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Image generation and editing tools backed by an OpenAI-compatible image API (or proxy).\n\
                Available tools:\n\
                - create: Generate an image from text. Required: `prompt`, `output_path`.\n\
                - edit: Edit images with a prompt. Required: `prompt`, `input_path`, `output_path`. No mask/inpainting.\n\
                - list_models: Returns available model names. Use as `model` param in create/edit.\n\
                - image_info: Inspect image file for format, dimensions, file size.\n\
                - image_resize: Resize to exact WIDTHxHEIGHT (stretches). Required: `input_path`, `size`, `output_path`."
                .to_string(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Format, ImageApiConfig, ImageDefaults};
    use rmcp::model::ContentBlock;

    fn sample_config() -> Config {
        Config {
            image_api: ImageApiConfig {
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
            },
            edit_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
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
            input_path: None,
            output_path: "/tmp/out.png".to_string(),
        });

        let result = server
            .create(params)
            .await
            .expect("create should return a CallToolResult, not an McpError");
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn create_tool_surfaces_output_path_validation_error() {
        let server = ImageMcpServer::new(sample_config());
        let params = Parameters(ImageParams {
            prompt: "a prompt".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            input_path: None,
            output_path: "".to_string(),
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
            input_path: None,
            output_path: "/tmp/out.png".to_string(),
        });

        let result = server
            .edit(params)
            .await
            .expect("edit should return a CallToolResult, not an McpError");
        assert_eq!(result.is_error, Some(true));
    }
}
