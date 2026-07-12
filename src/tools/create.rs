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

    super::respond_with_images(
        images,
        resolved.format,
        resolved.save,
        resolved.save_path.as_deref(),
        &config.payload_limits,
    )
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
                save: false,
            },
            edit_defaults: ImageDefaults {
                model: "test-model".to_string(),
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

    fn sample_params(prompt: &str) -> ImageParams {
        ImageParams {
            prompt: prompt.to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image: None,
            image_path: None,
            save: None,
            save_path: None,
        }
    }

    #[tokio::test]
    async fn empty_prompt_returns_validation_error_without_network_call() {
        let config = config_for_base_url("http://localhost:4000");
        let client = LiteLlmClient::new(&config.lite_llm);

        let result = run(&config, &client, sample_params("   ")).await;
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
        let client = LiteLlmClient::new(&config.lite_llm);

        let mut params = sample_params("a red bicycle");
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
    async fn successful_create_returns_inline_image_block() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": "aGVsbG8=" }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = LiteLlmClient::new(&config.lite_llm);

        let result = run(&config, &client, sample_params("a red bicycle")).await;
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        assert!(matches!(result.content[0], ContentBlock::Image(_)));
    }

    #[tokio::test]
    async fn litellm_api_error_surfaces_as_create_failed() {
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
        let client = LiteLlmClient::new(&config.lite_llm);

        let result = run(&config, &client, sample_params("a red bicycle")).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("create failed"));
    }

    #[tokio::test]
    async fn successful_create_with_save_writes_file() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        // Minimal valid PNG header plus payload, encoded as base64 so the
        // image_store format check passes.
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
        let client = LiteLlmClient::new(&config.lite_llm);

        let mut params = sample_params("a red bicycle");
        params.save = Some(true);

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        let path = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(path.ends_with(".png"));
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn successful_create_with_save_path_writes_to_requested_file() {
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
        let client = LiteLlmClient::new(&config.lite_llm);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let target = dir.join("bicycle.png");

        let mut params = sample_params("a red bicycle");
        params.save = Some(true);
        params.save_path = Some(target.display().to_string());

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
