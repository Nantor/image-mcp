use std::path::Path;

use rmcp::model::{CallToolResult, ContentBlock};

use crate::config::Config;
use crate::litellm::ImageApiClient;

use super::ImageParams;

/// Runs the `edit` (prompt-driven image editing) tool: resolves params
/// against `edit_defaults`, reads the required input image(s) from disk
/// via `input_path`, calls LiteLLM's `/v1/images/edits`, and writes the
/// result(s) to `output_path`.
pub async fn run(config: &Config, client: &ImageApiClient, params: ImageParams) -> CallToolResult {
    let has_input_path = params.input_path.as_ref().is_some_and(|v| !v.is_empty());

    if !has_input_path {
        return CallToolResult::error(vec![ContentBlock::text(
            "edit requires an `input_path` parameter (at least one path to an input image file)",
        )]);
    }

    let paths = params.input_path.clone().unwrap_or_default();
    let mut image_bytes_list = Vec::with_capacity(paths.len());
    for path in &paths {
        let p = Path::new(path);
        match p.symlink_metadata() {
            Ok(meta) if meta.is_symlink() => {
                return CallToolResult::error(vec![ContentBlock::text(format!(
                    "`input_path` entry {path:?} is a symlink; symlinks are not allowed for security reasons"
                ))]);
            }
            Err(err) => {
                return CallToolResult::error(vec![ContentBlock::text(format!(
                    "failed to check `input_path` entry {path:?}: {err}"
                ))]);
            }
            Ok(_) => {}
        }
        if path.contains("..") {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "`input_path` entry {path:?} contains '..'; path traversal is not allowed"
            ))]);
        }
        match std::fs::read(path) {
            Ok(bytes) => {
                if bytes.is_empty() {
                    return CallToolResult::error(vec![ContentBlock::text(format!(
                        "`input_path` entry {path:?} is empty; provide a valid image file"
                    ))]);
                }
                image_bytes_list.push(bytes);
            }
            Err(err) => {
                return CallToolResult::error(vec![ContentBlock::text(format!(
                    "failed to read `input_path` entry {path:?}: {err}"
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

    super::respond_with_images(images, resolved.format, &resolved.output_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Format, ImageDefaults, LiteLlmConfig};
    use base64::Engine as _;

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
            },
            edit_defaults: ImageDefaults {
                model: "test-model".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
            },
        }
    }

    fn sample_params(input_path: Option<Vec<String>>) -> ImageParams {
        ImageParams {
            prompt: "edit this".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            input_path,
            output_path: "/tmp/out.png".to_string(),
        }
    }

    #[tokio::test]
    async fn missing_input_path_parameter_returns_error() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.lite_llm);

        let result = run(&config, &client, sample_params(None)).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("edit requires an `input_path` parameter"));
    }

    #[tokio::test]
    async fn empty_prompt_returns_validation_error_without_network_call() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.lite_llm);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let file = dir.join("input.png");
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        std::fs::write(&file, &bytes).expect("write input file");

        let mut params = sample_params(Some(vec![file.display().to_string()]));
        params.prompt = "   ".to_string();

        let result = run(&config, &client, params).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("prompt"));
    }

    #[tokio::test]
    async fn image_api_error_surfaces_as_edit_failed() {
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
        let client = ImageApiClient::new(&config.lite_llm);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let file = dir.join("input.png");
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        std::fs::write(&file, &bytes).expect("write input file");

        let params = sample_params(Some(vec![file.display().to_string()]));

        let result = run(&config, &client, params).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("edit failed"));
    }

    #[tokio::test]
    async fn input_path_nonexistent_file_returns_error() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.lite_llm);

        let params = sample_params(Some(vec![
            "/tmp/definitely-does-not-exist-image-mcp.png".to_string(),
        ]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("failed to check `input_path` entry"));
    }

    #[tokio::test]
    async fn input_path_empty_file_content_returns_error() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.lite_llm);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let file = dir.join("empty.png");
        std::fs::write(&file, b"").expect("write empty file");

        let params = sample_params(Some(vec![file.display().to_string()]));

        let result = run(&config, &client, params).await;
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("is empty"));
    }

    #[tokio::test]
    async fn input_path_symlink_returns_error() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.lite_llm);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let real_file = dir.join("real.png");
        std::fs::write(&real_file, b"fake-png-data").expect("write real file");
        let symlink_file = dir.join("link.png");
        std::os::unix::fs::symlink(&real_file, &symlink_file).expect("create symlink");

        let params = sample_params(Some(vec![symlink_file.display().to_string()]));

        let result = run(&config, &client, params).await;
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("symlink"));
    }

    #[tokio::test]
    async fn input_path_with_dotdot_returns_error() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.lite_llm);

        let params = sample_params(Some(vec!["../../../etc/passwd".to_string()]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));
        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains(".."));
    }

    #[tokio::test]
    async fn empty_input_path_list_returns_error() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.lite_llm);

        let params = sample_params(Some(vec![]));

        let result = run(&config, &client, params).await;
        assert_eq!(result.is_error, Some(true));

        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("edit requires an `input_path` parameter"));
    }

    #[tokio::test]
    async fn successful_edit_with_input_path_writes_output_file() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let mut out_bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        out_bytes.extend_from_slice(b"rest-of-file");
        let out_b64 = base64::engine::general_purpose::STANDARD.encode(&out_bytes);

        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": out_b64 }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = ImageApiClient::new(&config.lite_llm);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let input_file = dir.join("input.png");
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        std::fs::write(&input_file, &bytes).expect("write input file");

        let mut params = sample_params(Some(vec![input_file.display().to_string()]));
        params.format = Some(Format::Png);
        let target = dir.join("output.png");
        params.output_path = target.display().to_string();

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

    #[tokio::test]
    async fn successful_edit_with_multiple_input_paths_sends_all_images() {
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
        let client = ImageApiClient::new(&config.lite_llm);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let mut paths = Vec::new();
        for name in ["a.png", "b.png"] {
            let file = dir.join(name);
            let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
            bytes.extend_from_slice(b"rest-of-file");
            std::fs::write(&file, &bytes).expect("write input file");
            paths.push(file.display().to_string());
        }

        let mut params = sample_params(Some(paths));
        params.output_path = dir.join("output.png").display().to_string();

        let result = run(&config, &client, params).await;
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
    }
}
