use rmcp::model::{CallToolResult, ContentBlock};

use crate::config::Config;

/// Runs the `list_models` tool. Never calls LiteLLM — just returns the
/// configured `image_models` list.
pub fn run(config: &Config) -> CallToolResult {
    let json = serde_json::json!({ "image_models": config.image_models });
    CallToolResult::success(vec![ContentBlock::text(json.to_string())])
}

#[cfg(test)]
mod tests {
    use crate::config::{ImageDefaults, LiteLlmConfig};

    use super::*;

    fn sample_config() -> Config {
        Config {
            lite_llm: LiteLlmConfig {
                base_url: "http://localhost:4000".to_string(),
                api_key: "test-key".to_string(),
            },
            image_models: vec!["gpt-image-1".to_string(), "dall-e-3".to_string()],
            create_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: crate::config::Format::Png,
                save: false,
            },
            edit_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: crate::config::Format::Jpg,
                save: false,
            },
        }
    }

    #[test]
    fn returns_image_models_field() {
        let config = sample_config();
        let result = run(&config);
        assert_eq!(result.is_error, Some(false));

        let content = &result.content[0];
        assert!(matches!(content, ContentBlock::Text(_)));
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };

        let parsed: serde_json::Value = serde_json::from_str(&text).expect("should be valid JSON");
        let models = parsed["image_models"]
            .as_array()
            .expect("image_models should be an array");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0], "gpt-image-1");
        assert_eq!(models[1], "dall-e-3");
    }

    #[test]
    fn returns_empty_models_list() {
        let mut config = sample_config();
        config.image_models.clear();

        let result = run(&config);
        assert_eq!(result.is_error, Some(false));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };

        let parsed: serde_json::Value = serde_json::from_str(&text).expect("should be valid JSON");
        let models = parsed["image_models"]
            .as_array()
            .expect("image_models should be an array");
        assert!(models.is_empty());
    }
}
