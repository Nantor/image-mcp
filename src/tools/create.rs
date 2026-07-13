use rmcp::model::{CallToolResult, ContentBlock};

use crate::config::Config;
use crate::litellm::ImageApiClient;

use super::ImageParams;

/// Runs the `create` (text-to-image) tool: resolves params against
/// `create_defaults`, calls LiteLLM's `/v1/images/generations`, and returns
/// either an inline image block or a saved file path per `save`.
pub async fn run(config: &Config, client: &ImageApiClient, params: ImageParams) -> CallToolResult {
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

    super::respond_with_images(images, resolved.format, &resolved.output_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Format, ImageDefaults, LiteLlmConfig};
    use base64::Engine as _;
    use rmcp::model::ContentBlock;

    fn config_for_base_url(base_url: &str) -> Config {
        Config {
            lite_llm: LiteLlmConfig {
                base_url: base_url.to_string(),
                api_key: "test-key".to_string(),
                request_timeout_secs: None,
            },
            image_models: vec!["test-model".to_string()],
            create_defaults: ImageDefaults {
                model: "test-model".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Png,
            },
            edit_defaults: ImageDefaults {
                model: "test-model".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
            },
        }
    }

    fn sample_params(prompt: &str, output_path: &str) -> ImageParams {
        ImageParams {
            prompt: prompt.to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            input_path: None,
            output_path: output_path.to_string(),
        }
    }

    #[tokio::test]
    async fn empty_prompt_returns_validation_error_without_network_call() {
        let config = config_for_base_url("http://localhost:4000");
        let client = ImageApiClient::new(&config.lite_llm);

        let result = run(&config, &client, sample_params("   ", "/tmp/out.png")).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("prompt"));
    }

    #[tokio::test]
    async fn invalid_size_returns_validation_error() {
        let config = config_for_base_url("http://localhost:4000");
        let client = ImageApiClient::new(&config.lite_llm);

        let mut params = sample_params("a red bicycle", "/tmp/out.png");
        params.size = Some("not-a-size".to_string());

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("`size`"));
    }

    #[tokio::test]
    async fn image_api_error_surfaces_as_create_failed() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"error":"bad request"}"#))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = ImageApiClient::new(&config.lite_llm);

        let result = run(
            &config,
            &client,
            sample_params("a red bicycle", "/tmp/out.png"),
        )
        .await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("create failed"));
    }

    #[tokio::test]
    async fn successful_create_writes_to_output_path() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let mut png_bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        png_bytes.extend_from_slice(b"rest-of-file");
        let png_b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": png_b64 }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = ImageApiClient::new(&config.lite_llm);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let target = dir.join("bicycle.png");

        let params = sample_params("a red bicycle", &target.display().to_string());

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        let path = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert_eq!(std::path::PathBuf::from(&path), target);
        std::fs::remove_dir_all(&dir).ok();
    }
}
