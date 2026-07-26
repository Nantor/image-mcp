use rmcp::model::{CallToolResult, ContentBlock};
use schemars::JsonSchema;
use serde::Deserialize;
use tracing;

use crate::config::Format;
use crate::image_ops;
use crate::image_store;

/// Params for the `image_crop` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageCropParams {
    /// Filesystem path to the input image file, read from disk.
    pub input_path: String,
    /// X coordinate (in pixels) of the crop box's top-left corner, measured
    /// from the image's left edge.
    pub x: u32,
    /// Y coordinate (in pixels) of the crop box's top-left corner, measured
    /// from the image's top edge.
    pub y: u32,
    /// Width, in pixels, of the crop box. Must be at least 1, and `x +
    /// width` must not exceed the source image's width.
    pub width: u32,
    /// Height, in pixels, of the crop box. Must be at least 1, and `y +
    /// height` must not exceed the source image's height.
    pub height: u32,
    /// Output image format. Defaults to the input image's own detected
    /// format if omitted.
    pub format: Option<Format>,
    /// Filesystem path to write the cropped image to. Required. The
    /// resolved format's extension is appended if the path has none.
    pub output_path: String,
}

/// Runs the `image_crop` tool: reads `input_path` from disk, cuts out the
/// rectangle at `(x, y)` sized `width`x`height`, and writes the result to
/// `output_path`. Never calls the upstream image API.
pub fn run(params: ImageCropParams) -> CallToolResult {
    if params.output_path.trim().is_empty() {
        return CallToolResult::error(vec![ContentBlock::text("`output_path` must not be empty")]);
    }
    if params.width == 0 || params.height == 0 {
        return CallToolResult::error(vec![ContentBlock::text(
            "`width` and `height` must each be at least 1",
        )]);
    }

    let bytes = match super::read_input_image(&params.input_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!("image_crop input_path rejected: {}", err);
            return CallToolResult::error(vec![ContentBlock::text(err)]);
        }
    };

    let input_format = match image_ops::detect_format(&bytes) {
        Ok(format) => format,
        Err(err) => {
            tracing::warn!(
                "image_crop failed to detect format for {}: {}",
                params.input_path,
                err
            );
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "failed to detect input image format: {err}"
            ))]);
        }
    };
    let output_format = params.format.unwrap_or(input_format);

    let cropped = match image_ops::crop(
        &bytes,
        input_format,
        params.x,
        params.y,
        params.width,
        params.height,
        output_format,
    ) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!("image_crop failed to crop {}: {}", params.input_path, err);
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "failed to crop image: {err}"
            ))]);
        }
    };

    let output_path = std::path::PathBuf::from(&params.output_path);
    match image_store::write_image_to_file(&cropped, &output_path, output_format) {
        Ok(path) => CallToolResult::success(vec![ContentBlock::text(path.display().to_string())]),
        Err(err) => {
            tracing::error!(
                "image_crop failed to write output {}: {}",
                output_path.display(),
                err
            );
            CallToolResult::error(vec![ContentBlock::text(format!(
                "failed to save cropped image: {err}"
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
    fn crops_to_exact_box_and_keeps_input_format() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 20, 30);
        let output = dir.join("out.png");

        let result = run(ImageCropParams {
            input_path: input,
            x: 5,
            y: 10,
            width: 8,
            height: 12,
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
        assert_eq!(info.width, 8);
        assert_eq!(info.height, 12);
        assert_eq!(info.format, Format::Png);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn crop_can_convert_format_via_format_param() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);
        let output = dir.join("out.jpg");

        let result = run(ImageCropParams {
            input_path: input,
            x: 0,
            y: 0,
            width: 5,
            height: 5,
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
    fn zero_width_returns_validation_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageCropParams {
            input_path: input,
            x: 0,
            y: 0,
            width: 0,
            height: 5,
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
    fn zero_height_returns_validation_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageCropParams {
            input_path: input,
            x: 0,
            y: 0,
            width: 5,
            height: 0,
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
    fn box_extending_past_edge_returns_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageCropParams {
            input_path: input,
            x: 5,
            y: 0,
            width: 6,
            height: 5,
            format: None,
            output_path: dir.join("out.png").display().to_string(),
        });
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.is_error, Some(true));
        let text = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(text.contains("outside"));
    }

    #[test]
    fn empty_output_path_returns_validation_error() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let input = write_image(&dir, "in.png", ImageFormat::Png, 10, 10);

        let result = run(ImageCropParams {
            input_path: input,
            x: 0,
            y: 0,
            width: 5,
            height: 5,
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
        let result = run(ImageCropParams {
            input_path: "/tmp/definitely-does-not-exist-image-mcp-crop.png".to_string(),
            x: 0,
            y: 0,
            width: 5,
            height: 5,
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

        let result = run(ImageCropParams {
            input_path: file.display().to_string(),
            x: 0,
            y: 0,
            width: 5,
            height: 5,
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

        let result = run(ImageCropParams {
            input_path: input,
            x: 0,
            y: 0,
            width: 5,
            height: 5,
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

        let result = run(ImageCropParams {
            input_path: symlink_file.display().to_string(),
            x: 0,
            y: 0,
            width: 5,
            height: 5,
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
