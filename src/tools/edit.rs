use base64::Engine as _;
use rmcp::model::{CallToolResult, ContentBlock};

use crate::config::Config;
use crate::litellm::LiteLlmClient;

use super::ImageParams;

/// Runs the `edit` (prompt-driven image editing) tool: resolves params
/// against `edit_defaults`, decodes the required input `image`(s), calls
/// LiteLLM's `/v1/images/edits`, and returns either an inline image block or
/// a saved file path per `save`.
pub async fn run(config: &Config, client: &LiteLlmClient, params: ImageParams) -> CallToolResult {
    let Some(image_b64s) = params.image.clone().filter(|images| !images.is_empty()) else {
        return CallToolResult::error(vec![ContentBlock::text(
            "edit requires an `image` parameter (at least one base64-encoded input image)",
        )]);
    };

    let mut image_bytes_list = Vec::with_capacity(image_b64s.len());
    for image_b64 in &image_b64s {
        match base64::engine::general_purpose::STANDARD.decode(image_b64) {
            Ok(bytes) => image_bytes_list.push(bytes),
            Err(err) => {
                return CallToolResult::error(vec![ContentBlock::text(format!(
                    "invalid base64 in `image` parameter: {err}"
                ))]);
            }
        }
    }

    let resolved = params.resolve(&config.edit_defaults);

    if let Err(err) = resolved.validate() {
        return CallToolResult::error(vec![ContentBlock::text(err)]);
    }

    let images = match client.edit(&resolved, image_bytes_list).await {
        Ok(images) => images,
        Err(err) => {
            return CallToolResult::error(vec![ContentBlock::text(format!("edit failed: {err}"))]);
        }
    };

    super::respond_with_images(images, resolved.format, resolved.save)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Format, ImageDefaults, LiteLlmConfig};

    fn sample_config() -> Config {
        config_for_base_url("http://localhost:4000")
    }

    fn config_for_base_url(base_url: &str) -> Config {
        Config {
            lite_llm: LiteLlmConfig {
                base_url: base_url.to_string(),
                api_key: "test-key".to_string(),
                request_timeout_secs: None,
            },
            image_models: vec![],
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
        }
    }

    fn sample_params(image: Option<Vec<String>>) -> ImageParams {
        ImageParams {
            prompt: "edit this".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image,
            save: None,
        }
    }

    #[tokio::test]
    async fn missing_image_parameter_returns_error() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        let result = run(&config, &client, sample_params(None)).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("edit requires an `image` parameter"));
    }

    #[tokio::test]
    async fn invalid_base64_returns_error() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = sample_params(Some(vec!["not-valid-base64!!!".to_string()]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("invalid base64"));
    }

    #[tokio::test]
    async fn valid_base64_but_empty_decodes_and_reaches_client() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // "" is valid base64 (decodes to empty bytes). This test verifies
        // the decode path works and the request actually reaches the
        // LiteLLM client (mocked here) rather than erroring earlier.
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": "ZWRpdGVk" }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = sample_params(Some(vec!["".to_string()]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(false));
    }

    #[tokio::test]
    async fn empty_image_list_returns_error() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = sample_params(Some(vec![]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("edit requires an `image` parameter"));
    }

    #[tokio::test]
    async fn second_image_invalid_base64_returns_error() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = sample_params(Some(vec![
            "".to_string(),
            "not-valid-base64!!!".to_string(),
        ]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("invalid base64"));
    }

    #[tokio::test]
    async fn empty_prompt_returns_validation_error_without_network_call() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        let mut params = sample_params(Some(vec!["aGVsbG8=".to_string()]));
        params.prompt = "   ".to_string();

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("prompt"));
    }

    #[tokio::test]
    async fn successful_edit_returns_inline_image_block() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": "ZWRpdGVk" }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = sample_params(Some(vec!["aGVsbG8=".to_string()]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        assert!(matches!(result.content[0], ContentBlock::Image(_)));
    }

    #[tokio::test]
    async fn litellm_api_error_surfaces_as_edit_failed() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"error":"bad request"}"#))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = sample_params(Some(vec!["aGVsbG8=".to_string()]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("edit failed"));
    }

    #[test]
    fn base64_padding_variants() {
        // Test that common base64 padding scenarios are detected correctly.
        // These are all invalid and should trigger the decode error path.

        // No padding at all (not always required but triggers error on some inputs)
        let result = base64::engine::general_purpose::STANDARD.decode("abc");
        assert!(result.is_err());

        // Over-padded
        let result = base64::engine::general_purpose::STANDARD.decode("YQ===");
        assert!(result.is_err());
    }
}
