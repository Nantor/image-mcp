pub mod create;
pub mod edit;
pub mod list_models;

use std::path::{Path, PathBuf};

use rmcp::model::{CallToolResult, ContentBlock};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::{Format, ImageDefaults};
use crate::image_store;

/// Params shared by the `create` and `edit` tools. Per-call values here
/// override the matching config default; anything left `None` falls back
/// to `create_defaults` / `edit_defaults` in the config.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageParams {
    /// Text prompt describing the desired image (or the edit to apply).
    pub prompt: String,
    /// Model to use. Falls back to the configured default for this mode.
    pub model: Option<String>,
    /// Number of images to generate. Falls back to the configured default.
    pub n: Option<u32>,
    /// Image size, e.g. "1024x1024". Falls back to the configured default.
    pub size: Option<String>,
    /// Output image format. Falls back to the configured default.
    pub format: Option<Format>,
    /// Filesystem path(s) to input image(s), read from disk. Required for
    /// `edit` (at least one entry); unused for `create`.
    pub input_path: Option<Vec<String>>,
    /// Filesystem path to write the output image to. Required. If `n`
    /// resolves to more than 1, each generated image is written next to
    /// this path with a `-1`, `-2`, ... suffix inserted before the
    /// extension (e.g. `out.png` becomes `out-1.png`, `out-2.png`, ...);
    /// with exactly one image, this exact path is used as-is. The
    /// resolved format's extension is appended if the path has none.
    pub output_path: String,
}

/// `ImageParams` merged with the mode's config defaults — every field is
/// resolved to a concrete value.
pub struct ResolvedParams {
    pub prompt: String,
    pub model: String,
    pub n: u32,
    pub size: String,
    pub format: Format,
    pub output_path: PathBuf,
}

impl ImageParams {
    pub fn resolve(self, defaults: &ImageDefaults) -> ResolvedParams {
        ResolvedParams {
            prompt: self.prompt,
            model: self.model.unwrap_or_else(|| defaults.model.clone()),
            n: self.n.unwrap_or(defaults.n),
            size: self.size.unwrap_or_else(|| defaults.size.clone()),
            format: self.format.unwrap_or(defaults.format),
            output_path: PathBuf::from(self.output_path),
        }
    }
}

impl ResolvedParams {
    /// Basic sanity checks run before hitting the network, so obviously
    /// invalid values surface as an immediate, clear tool error instead of
    /// round-tripping to LiteLLM for a less helpful API error.
    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.trim().is_empty() {
            return Err("`prompt` must not be empty".to_string());
        }
        if self.n == 0 {
            return Err("`n` must be at least 1".to_string());
        }
        if !is_valid_size(&self.size) {
            return Err(format!(
                "`size` must be in the form WIDTHxHEIGHT (e.g. \"1024x1024\"), got {:?}",
                self.size
            ));
        }
        if self.output_path.as_os_str().is_empty() {
            return Err("`output_path` must not be empty".to_string());
        }
        Ok(())
    }
}

/// Checks that `size` looks like `<digits>x<digits>` (e.g. `1024x1024`).
/// This is a shape check only — the actual dimensions are still validated
/// by LiteLLM/the model, since supported sizes vary per model.
fn is_valid_size(size: &str) -> bool {
    match size.split_once('x') {
        Some((w, h)) => {
            !w.is_empty()
                && !h.is_empty()
                && w.chars().all(|c| c.is_ascii_digit())
                && h.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Shared response handling for `create` and `edit`: writes each returned
/// image to disk at `output_path` (or a `-<n>`-suffixed variant of it, for
/// n>1) and returns the written filename(s) as text content.
pub fn respond_with_images(
    images: Vec<String>,
    format: Format,
    output_path: &Path,
) -> CallToolResult {
    let mut content = Vec::with_capacity(images.len());
    let total = images.len();

    for (index, b64_data) in images.into_iter().enumerate() {
        let target = if total > 1 {
            suffixed_path(output_path, index + 1)
        } else {
            output_path.to_path_buf()
        };

        match image_store::save_image(&b64_data, format, &target) {
            Ok(path) => content.push(ContentBlock::text(path.display().to_string())),
            Err(err) => {
                return CallToolResult::error(vec![ContentBlock::text(format!(
                    "failed to save image: {err}"
                ))]);
            }
        }
    }

    CallToolResult::success(content)
}

/// Inserts a `-<index>` suffix before the file extension (or at the end, if
/// there is no extension) of `path`, so that multiple images written to
/// the same `output_path` don't overwrite each other. `index` is 1-based
/// (the first of several images becomes `-1`, not unsuffixed).
fn suffixed_path(path: &Path, index: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let new_name = match path.extension() {
        Some(ext) => format!("{stem}-{index}.{}", ext.to_string_lossy()),
        None => format!("{stem}-{index}"),
    };
    path.with_file_name(new_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_defaults() -> ImageDefaults {
        ImageDefaults {
            model: "default-model".to_string(),
            n: 1,
            size: "1024x1024".to_string(),
            format: Format::Png,
        }
    }

    #[test]
    fn resolve_all_defaults() {
        let params = ImageParams {
            prompt: "hello".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            input_path: None,
            output_path: "/tmp/out.png".to_string(),
        };
        let defaults = sample_defaults();
        let resolved = params.resolve(&defaults);

        assert_eq!(resolved.model, "default-model");
        assert_eq!(resolved.n, 1);
        assert_eq!(resolved.size, "1024x1024");
        assert_eq!(resolved.format, Format::Png);
        assert_eq!(resolved.prompt, "hello");
        assert_eq!(resolved.output_path, PathBuf::from("/tmp/out.png"));
    }

    #[test]
    fn resolve_all_overrides() {
        let params = ImageParams {
            prompt: "hello".to_string(),
            model: Some("custom-model".to_string()),
            n: Some(4),
            size: Some("2048x2048".to_string()),
            format: Some(Format::Jpg),
            input_path: None,
            output_path: "/tmp/out.jpg".to_string(),
        };
        let defaults = sample_defaults();
        let resolved = params.resolve(&defaults);

        assert_eq!(resolved.model, "custom-model");
        assert_eq!(resolved.n, 4);
        assert_eq!(resolved.size, "2048x2048");
        assert_eq!(resolved.format, Format::Jpg);
        assert_eq!(resolved.prompt, "hello");
    }

    #[test]
    fn resolve_partial_override() {
        let params = ImageParams {
            prompt: "hello".to_string(),
            model: Some("overridden".to_string()),
            n: None,
            size: None,
            format: Some(Format::Webp),
            input_path: None,
            output_path: "/tmp/out.webp".to_string(),
        };
        let defaults = ImageDefaults {
            model: "default-model".to_string(),
            n: 3,
            size: "512x512".to_string(),
            format: Format::Png,
        };
        let resolved = params.resolve(&defaults);

        assert_eq!(resolved.model, "overridden");
        assert_eq!(resolved.n, 3);
        assert_eq!(resolved.size, "512x512");
        assert_eq!(resolved.format, Format::Webp);
    }

    #[test]
    fn resolve_prompt_always_came_from_params() {
        // prompt has no Option variant, it always comes from params
        let params = ImageParams {
            prompt: "my prompt".to_string(),
            model: Some("m".into()),
            n: None,
            size: None,
            format: None,
            input_path: None,
            output_path: "/tmp/out.png".to_string(),
        };
        let defaults = sample_defaults();
        let resolved = params.resolve(&defaults);
        assert_eq!(resolved.prompt, "my prompt");
    }

    #[test]
    fn resolve_maps_output_path_string_to_path_buf() {
        let params = ImageParams {
            prompt: "hello".to_string(),
            model: None,
            n: None,
            size: None,
            format: None,
            input_path: None,
            output_path: "/tmp/out.png".to_string(),
        };
        let resolved = params.resolve(&sample_defaults());
        assert_eq!(resolved.output_path, PathBuf::from("/tmp/out.png"));
    }

    fn sample_resolved() -> ResolvedParams {
        ResolvedParams {
            prompt: "a prompt".to_string(),
            model: "model".to_string(),
            n: 1,
            size: "1024x1024".to_string(),
            format: Format::Png,
            output_path: PathBuf::from("/tmp/out.png"),
        }
    }

    #[test]
    fn validate_accepts_sane_params() {
        assert!(sample_resolved().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_prompt() {
        let mut resolved = sample_resolved();
        resolved.prompt = "   ".to_string();
        let err = resolved.validate().unwrap_err();
        assert!(err.contains("prompt"));
    }

    #[test]
    fn validate_rejects_zero_n() {
        let mut resolved = sample_resolved();
        resolved.n = 0;
        let err = resolved.validate().unwrap_err();
        assert!(err.contains("`n`"));
    }

    #[test]
    fn validate_rejects_malformed_size() {
        for bad in ["1024", "1024x", "x1024", "1024x1024x1024", "wxh", ""] {
            let mut resolved = sample_resolved();
            resolved.size = bad.to_string();
            assert!(
                resolved.validate().is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn validate_accepts_various_valid_sizes() {
        for good in ["1024x1024", "512x768", "1x1"] {
            let mut resolved = sample_resolved();
            resolved.size = good.to_string();
            assert!(
                resolved.validate().is_ok(),
                "expected {good:?} to be accepted"
            );
        }
    }

    #[test]
    fn validate_rejects_empty_output_path() {
        let mut resolved = sample_resolved();
        resolved.output_path = PathBuf::from("");
        let err = resolved.validate().unwrap_err();
        assert!(err.contains("output_path"));
    }

    #[test]
    fn validate_accepts_non_empty_output_path() {
        let mut resolved = sample_resolved();
        resolved.output_path = PathBuf::from("/tmp/out.png");
        assert!(resolved.validate().is_ok());
    }

    fn valid_png_b64() -> String {
        use base64::Engine as _;
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    }

    #[test]
    fn respond_with_images_writes_and_returns_text_paths() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("out.png");

        let result = respond_with_images(vec![valid_png_b64()], Format::Png, &file);
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        let path = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        assert_eq!(PathBuf::from(&path), file);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn respond_with_images_errors_on_invalid_base64() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("out.png");

        let result =
            respond_with_images(vec!["not valid base64!!!".to_string()], Format::Png, &file);
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn respond_with_images_multiple_images_get_numbered_suffixes() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("out.png");

        let result =
            respond_with_images(vec![valid_png_b64(), valid_png_b64()], Format::Png, &file);
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 2);

        let paths: Vec<PathBuf> = result
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text(t) => PathBuf::from(&t.text),
                _ => panic!("expected text block"),
            })
            .collect();
        assert_eq!(paths[0], dir.join("out-1.png"));
        assert_eq!(paths[1], dir.join("out-2.png"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn suffixed_path_inserts_before_extension() {
        let path = PathBuf::from("/tmp/foo/bar.png");
        assert_eq!(suffixed_path(&path, 2), PathBuf::from("/tmp/foo/bar-2.png"));
    }

    #[test]
    fn suffixed_path_handles_missing_extension() {
        let path = PathBuf::from("/tmp/foo/bar");
        assert_eq!(suffixed_path(&path, 1), PathBuf::from("/tmp/foo/bar-1"));
    }
}
