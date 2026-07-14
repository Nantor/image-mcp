use rmcp::model::{CallToolResult, ContentBlock};
use schemars::JsonSchema;
use serde::Deserialize;
use tracing;

use crate::image_ops;

/// Params for the `image_info` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageInfoParams {
    /// Filesystem path to the image file to inspect, read from disk.
    pub input_path: String,
}

/// Runs the `image_info` tool: reads `input_path` from disk and reports
/// its detected image type, pixel dimensions, and file size in bytes.
/// Read-only — never calls the upstream image API.
pub fn run(params: ImageInfoParams) -> CallToolResult {
    let bytes = match super::read_input_image(&params.input_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!("image_info input_path rejected: {}", err);
            return CallToolResult::error(vec![ContentBlock::text(err)]);
        }
    };

    let info = match image_ops::inspect(&bytes) {
        Ok(info) => info,
        Err(err) => {
            tracing::warn!(
                "image_info failed to inspect {}: {}",
                params.input_path,
                err
            );
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "failed to inspect image: {err}"
            ))]);
        }
    };

    let json = serde_json::json!({
        "format": info.format.as_str(),
        "width": info.width,
        "height": info.height,
        "size_bytes": bytes.len(),
    });
    CallToolResult::success(vec![ContentBlock::text(json.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageFormat;
    use std::io::Cursor;

    fn write_image(
        dir: &std::path::Path,
        name: &str,
        format: ImageFormat,
        w: u32,
        h: u32,
    ) -> String {
        std::fs::create_dir_all(dir).expect("create test dir");
        let file = dir.join(name);
        let img = image::DynamicImage::new_rgb8(w, h);
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, format).unwrap();
        std::fs::write(&file, buf.into_inner()).expect("write test image");
        file.display().to_string()
    }

    #[test]
    fn returns_format_dimensions_and_size_for_png() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let path = write_image(&dir, "in.png", ImageFormat::Png, 37, 51);

        let result = run(ImageInfoParams { input_path: path });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(false));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["format"], "png");
        assert_eq!(parsed["width"], 37);
        assert_eq!(parsed["height"], 51);
        assert!(parsed["size_bytes"].as_u64().unwrap() > 0);
    }

    #[test]
    fn returns_format_for_jpeg() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let path = write_image(&dir, "in.jpg", ImageFormat::Jpeg, 10, 20);

        let result = run(ImageInfoParams { input_path: path });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(false));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["format"], "jpg");
        assert_eq!(parsed["width"], 10);
        assert_eq!(parsed["height"], 20);
    }

    #[test]
    fn returns_format_for_webp() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let path = write_image(&dir, "in.webp", ImageFormat::WebP, 8, 8);

        let result = run(ImageInfoParams { input_path: path });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(false));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["format"], "webp");
    }

    #[test]
    fn nonexistent_file_returns_error() {
        let result = run(ImageInfoParams {
            input_path: "/tmp/definitely-does-not-exist-image-mcp-info.png".to_string(),
        });
        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("failed to check `input_path` entry"));
    }

    #[test]
    fn unrecognized_format_returns_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let file = dir.join("not-an-image.png");
        std::fs::write(&file, b"this is definitely not image data").expect("write file");

        let result = run(ImageInfoParams {
            input_path: file.display().to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("failed to inspect image"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_input_path_returns_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let real_file = dir.join("real.png");
        std::fs::write(&real_file, b"fake-png-data").expect("write real file");
        let symlink_file = dir.join("link.png");
        std::os::unix::fs::symlink(&real_file, &symlink_file).expect("create symlink");

        let result = run(ImageInfoParams {
            input_path: symlink_file.display().to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("symlink"));
    }
}
