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
        Config {
            lite_llm: LiteLlmConfig {
                base_url: "http://localhost:4000".to_string(),
                api_key: "test-key".to_string(),
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

    #[tokio::test]
    async fn missing_image_parameter_returns_error() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = ImageParams {
            prompt: "edit this".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image: None,
            save: None,
        };

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
    async fn invalid_base64_returns_error() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = ImageParams {
            prompt: "edit this".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image: Some(vec!["not-valid-base64!!!".to_string()]),
            save: None,
        };

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
    async fn valid_base64_but_empty_returns_error() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        // "" is valid base64 (decodes to empty bytes), but it will
        // reach the client.edit() call which will fail. This test
        // verifies the decode path works.
        let params = ImageParams {
            prompt: "edit this".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image: Some(vec!["".to_string()]),
            save: None,
        };

        // base64::STANDARD.decode("") returns Ok(empty vec), so we proceed
        // to client.edit() which hits the network and errors (since there
        // is no real LiteLLM proxy at localhost:4000). We just assert
        // isError=true regardless of the specific error message.
        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn empty_image_list_returns_error() {
        let config = sample_config();
        let client = LiteLlmClient::new(&config.lite_llm);

        let params = ImageParams {
            prompt: "edit this".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image: Some(vec![]),
            save: None,
        };

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

        let params = ImageParams {
            prompt: "edit this".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            image: Some(vec!["".to_string(), "not-valid-base64!!!".to_string()]),
            save: None,
        };

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("invalid base64"));
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
