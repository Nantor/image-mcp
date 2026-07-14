use rmcp::model::{CallToolResult, ContentBlock};
use schemars::JsonSchema;
use serde::Deserialize;
use tracing;

use crate::config::Format;
use crate::image_ops;
use crate::image_store;

/// Maximum width or height, in pixels, accepted for a resize target.
/// `size` is caller-controlled input; without a bound, a request like
/// `50000x50000` would make the `image` crate allocate gigabytes of
/// memory and spend tens of seconds resampling, a local
/// denial-of-service. This cap is generous for any realistic image
/// editing use case while keeping worst-case memory/CPU bounded.
const MAX_DIMENSION: u32 = 8192;

/// Params for the `image_resize` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageResizeParams {
    /// Filesystem path to the input image file, read from disk.
    pub input_path: String,
    /// Target size, in the form `WIDTHxHEIGHT` (e.g. "512x512"). The image
    /// is stretched to exactly this size; the aspect ratio is not
    /// preserved. Each dimension must be between 1 and 8192 pixels.
    pub size: String,
    /// Output image format. Defaults to the input image's own detected
    /// format if omitted.
    pub format: Option<Format>,
    /// Filesystem path to write the resized image to. Required. The
    /// resolved format's extension is appended if the path has none.
    pub output_path: String,
}

/// Runs the `image_resize` tool: reads `input_path` from disk, resizes it
/// to exactly `size` (stretching to fit, not preserving aspect ratio), and
/// writes the result to `output_path`. Never calls the upstream image API.
pub fn run(params: ImageResizeParams) -> CallToolResult {
    if params.output_path.trim().is_empty() {
        return CallToolResult::error(vec![ContentBlock::text("`output_path` must not be empty")]);
    }

    let (width, height) = match crate::tools::parse_size(&params.size) {
        Ok(dims) => dims,
        Err(err) => {
            tracing::warn!("image_resize validation error: {}", err);
            return CallToolResult::error(vec![ContentBlock::text(err)]);
        }
    };
    if width == 0 || height == 0 {
        return CallToolResult::error(vec![ContentBlock::text(
            "`size` width and height must each be at least 1",
        )]);
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return CallToolResult::error(vec![ContentBlock::text(format!(
            "`size` width and height must each be at most {MAX_DIMENSION} pixels, got {width}x{height}"
        ))]);
    }

    let bytes = match super::read_input_image(&params.input_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!("image_resize input_path rejected: {}", err);
            return CallToolResult::error(vec![ContentBlock::text(err)]);
        }
    };

    let input_format = match image_ops::detect_format(&bytes) {
        Ok(format) => format,
        Err(err) => {
            tracing::warn!(
                "image_resize failed to detect format for {}: {}",
                params.input_path,
                err
            );
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "failed to detect input image format: {err}"
            ))]);
        }
    };
    let output_format = params.format.unwrap_or(input_format);

    let resized = match image_ops::resize(&bytes, input_format, width, height, output_format) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(
                "image_resize failed to resize {}: {}",
                params.input_path,
                err
            );
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "failed to resize image: {err}"
            ))]);
        }
    };

    let output_path = std::path::PathBuf::from(&params.output_path);
    match image_store::write_image_to_file(&resized, &output_path, output_format) {
        Ok(path) => CallToolResult::success(vec![ContentBlock::text(path.display().to_string())]),
        Err(err) => {
            tracing::error!(
                "image_resize failed to write output {}: {}",
                output_path.display(),
                err
            );
            CallToolResult::error(vec![ContentBlock::text(format!(
                "failed to save resized image: {err}"
            ))])
        }
    }
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
    fn resizes_to_exact_dimensions_and_keeps_input_format() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 20);
        let output = dir.join("out.png");

        let result = run(ImageResizeParams {
            input_path: input,
            size: "40x5".to_string(),
            format: None,
            output_path: output.display().to_string(),
        });

        assert_eq!(result.is_error, Some(false));
        let path = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        let written = std::fs::read(&path).expect("output file should exist");
        let info = image_ops::inspect(&written).unwrap();
        assert_eq!(info.width, 40);
        assert_eq!(info.height, 5);
        assert_eq!(info.format, Format::Png);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resize_can_convert_format_via_format_param() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);
        let output = dir.join("out.jpg");

        let result = run(ImageResizeParams {
            input_path: input,
            size: "20x20".to_string(),
            format: Some(Format::Jpg),
            output_path: output.display().to_string(),
        });

        assert_eq!(result.is_error, Some(false));
        let path = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        let written = std::fs::read(&path).expect("output file should exist");
        let info = image_ops::inspect(&written).unwrap();
        assert_eq!(info.format, Format::Jpg);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_size_returns_validation_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageResizeParams {
            input_path: input,
            size: "not-a-size".to_string(),
            format: None,
            output_path: dir.join("out.png").display().to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("`size`"));
    }

    #[test]
    fn zero_width_returns_validation_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageResizeParams {
            input_path: input,
            size: "0x10".to_string(),
            format: None,
            output_path: dir.join("out.png").display().to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("at least 1"));
    }

    #[test]
    fn oversized_dimension_returns_validation_error_without_allocating() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageResizeParams {
            input_path: input,
            size: "50000x50000".to_string(),
            format: None,
            output_path: dir.join("out.png").display().to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("at most"));
    }

    #[test]
    fn dimension_at_max_is_accepted_by_validation() {
        // Only checks that MAX_DIMENSION itself doesn't trip the "too
        // large" validation error; doesn't actually resize to it (that
        // would be slow/memory-heavy in a test).
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 2, 2);

        let result = run(ImageResizeParams {
            input_path: input,
            size: format!("{MAX_DIMENSION}x1"),
            format: None,
            output_path: dir.join("out.png").display().to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        // Should not fail with the "too large" validation error; it may
        // succeed or fail for other reasons, but not that one.
        if result.is_error == Some(true) {
            let text = match &result.content[0] {
                ContentBlock::Text(t) => t.text.clone(),
                _ => panic!("expected text block"),
            };
            assert!(!text.contains("at most"));
        }
    }

    #[test]
    fn empty_output_path_returns_validation_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageResizeParams {
            input_path: input,
            size: "10x10".to_string(),
            format: None,
            output_path: "".to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("output_path"));
    }

    #[test]
    fn nonexistent_input_returns_error() {
        let result = run(ImageResizeParams {
            input_path: "/tmp/definitely-does-not-exist-image-mcp-resize.png".to_string(),
            size: "10x10".to_string(),
            format: None,
            output_path: "/tmp/out.png".to_string(),
        });
        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("failed to check `input_path` entry"));
    }

    #[test]
    fn unrecognized_input_format_returns_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let file = dir.join("not-an-image.png");
        std::fs::write(&file, b"not an image at all").expect("write file");

        let result = run(ImageResizeParams {
            input_path: file.display().to_string(),
            size: "10x10".to_string(),
            format: None,
            output_path: dir.join("out.png").display().to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("failed to detect input image format"));
    }

    #[test]
    fn output_path_missing_extension_gets_format_extension_appended() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageResizeParams {
            input_path: input,
            size: "5x5".to_string(),
            format: None,
            output_path: dir.join("out-no-ext").display().to_string(),
        });

        assert_eq!(result.is_error, Some(false));
        let path = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(path.ends_with("out-no-ext.png"));
        std::fs::remove_dir_all(&dir).ok();
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

        let result = run(ImageResizeParams {
            input_path: symlink_file.display().to_string(),
            size: "10x10".to_string(),
            format: None,
            output_path: dir.join("out.png").display().to_string(),
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
