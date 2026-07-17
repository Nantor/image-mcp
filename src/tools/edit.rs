use rmcp::model::{CallToolResult, ContentBlock};
use tracing;

use crate::config::Config;
use crate::image_api::ImageApiClient;

use super::ImageParams;

/// Runs the `edit` (prompt-driven image editing) tool: resolves params
/// against `edit_defaults`, reads the required input image(s) from disk
/// via `input_path`, calls the image API's `/v1/images/edits`, and writes the
/// result(s) to `output_path`.
pub async fn run(config: &Config, client: &ImageApiClient, params: ImageParams) -> CallToolResult {
    let has_input_path = params.input_path.as_ref().is_some_and(|v| !v.is_empty());

    if !has_input_path {
        tracing::warn!("edit called without input_path parameter");
        return CallToolResult::error(vec![ContentBlock::text(
            "edit requires an `input_path` parameter (at least one path to an input image file)",
        )]);
    }

    let paths = params.input_path.clone().unwrap_or_default();
    let mut image_bytes_list = Vec::with_capacity(paths.len());
    for path in &paths {
        match super::read_input_image(path) {
            Ok(bytes) => image_bytes_list.push(bytes),
            Err(err) => {
                tracing::warn!("edit input_path entry {} rejected: {}", path, err);
                return CallToolResult::error(vec![ContentBlock::text(err)]);
            }
        }
    }

    let resolved = params.resolve(&config.edit_defaults);

    if let Err(err) = resolved.validate() {
        tracing::warn!("edit validation error: {}", err);
        return CallToolResult::error(vec![ContentBlock::text(err)]);
    }

    let images = match client.edit(&resolved, image_bytes_list).await {
        Ok(images) => images,
        Err(err) => {
            tracing::error!("edit API error on model={}: {}", resolved.model, err);
            return CallToolResult::error(vec![ContentBlock::text(format!("edit failed: {err}"))]);
        }
    };

    super::respond_with_images(images, resolved.format, &resolved.output_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Format, ImageApiConfig, ImageDefaults};
    use base64::Engine as _;

    fn sample_config() -> Config {
        config_for_base_url("http://localhost:4000")
    }

    fn config_for_base_url(base_url: &str) -> Config {
        Config {
            image_api: ImageApiConfig {
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
        let client = ImageApiClient::new(&config.image_api);

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
        let client = ImageApiClient::new(&config.image_api);

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
        let client = ImageApiClient::new(&config.image_api);

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
        let client = ImageApiClient::new(&config.image_api);

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
        let client = ImageApiClient::new(&config.image_api);

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

    #[cfg(unix)]
    #[tokio::test]
    async fn input_path_symlink_returns_error() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.image_api);

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
        let client = ImageApiClient::new(&config.image_api);

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
    async fn input_path_with_legitimate_dotdot_in_filename_is_not_blocked() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.image_api);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let file = dir.join("my..config.png");
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        std::fs::write(&file, &bytes).expect("write input file");

        let mut params = sample_params(Some(vec![file.display().to_string()]));
        params.prompt = "test".to_string();
        params.output_path = dir.join("out.png").display().to_string();

        let result = run(&config, &client, params).await;
        std::fs::remove_dir_all(&dir).ok();

        // Not an error for traversal; fails at HTTP layer as expected
        assert_eq!(result.is_error, Some(true));
        let content = &result.content[0];
        let text = match content {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(
            !text.contains("path traversal"),
            "legitimate filename with '..' was incorrectly blocked as path traversal"
        );
    }

    #[tokio::test]
    async fn empty_input_path_list_returns_error() {
        let config = sample_config();
        let client = ImageApiClient::new(&config.image_api);

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
        use image::ImageFormat;
        use std::io::Cursor;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let mut out_buf = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(64, 64)
            .write_to(&mut out_buf, ImageFormat::Png)
            .unwrap();
        let out_b64 = base64::engine::general_purpose::STANDARD.encode(out_buf.into_inner());

        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": out_b64 }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = ImageApiClient::new(&config.image_api);

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
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            std::path::PathBuf::from(parsed["path"].as_str().unwrap()),
            target
        );
        assert!(parsed["width"].is_u64());
        assert!(parsed["height"].is_u64());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn edit_with_multiple_input_paths_wrong_format_fails() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let resp_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nfake-png");

        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": resp_b64 }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = ImageApiClient::new(&config.image_api);

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

    #[tokio::test]
    async fn successful_edit_with_multiple_input_paths_sends_all_images() {
        use image::ImageFormat;
        use std::io::Cursor;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(64, 64)
            .write_to(&mut buf, ImageFormat::Png)
            .unwrap();
        let resp_b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());

        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": resp_b64 }],
            })))
            .mount(&mock_server)
            .await;

        let config = config_for_base_url(&mock_server.uri());
        let client = ImageApiClient::new(&config.image_api);

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
        params.format = Some(Format::Png);
        let target = dir.join("output.png");
        params.output_path = target.display().to_string();

        let result = run(&config, &client, params).await;

        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            std::path::PathBuf::from(parsed["path"].as_str().unwrap()),
            target
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
