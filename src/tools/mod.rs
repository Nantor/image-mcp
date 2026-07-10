pub mod create;
pub mod edit;
pub mod list_models;

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
    /// Base64-encoded input image. Required for `edit`, unused for `create`.
    pub image: Option<String>,
    /// If true, write the image to disk and return its path instead of an
    /// inline image content block. Falls back to the configured default.
    pub save: Option<bool>,
}

/// `ImageParams` merged with the mode's config defaults — every field is
/// resolved to a concrete value.
pub struct ResolvedParams {
    pub prompt: String,
    pub model: String,
    pub n: u32,
    pub size: String,
    pub format: Format,
    pub save: bool,
}

impl ImageParams {
    pub fn resolve(self, defaults: &ImageDefaults) -> ResolvedParams {
        ResolvedParams {
            prompt: self.prompt,
            model: self.model.unwrap_or_else(|| defaults.model.clone()),
            n: self.n.unwrap_or(defaults.n),
            size: self.size.unwrap_or_else(|| defaults.size.clone()),
            format: self.format.unwrap_or(defaults.format),
            save: self.save.unwrap_or(defaults.save),
        }
    }
}

/// Shared response handling for `create` and `edit`: either writes each
/// image to disk and returns its path as text, or returns it inline as an
/// MCP `image` content block, depending on `save`.
pub fn respond_with_images(images: Vec<String>, format: Format, save: bool) -> CallToolResult {
    let mut content = Vec::with_capacity(images.len());

    for b64_data in images {
        if save {
            match image_store::save_image(&b64_data, format) {
                Ok(path) => content.push(ContentBlock::text(path.display().to_string())),
                Err(err) => {
                    return CallToolResult::error(vec![ContentBlock::text(format!(
                        "failed to save image: {err}"
                    ))]);
                }
            }
        } else {
            content.push(ContentBlock::image(b64_data, format.mime_type()));
        }
    }

    CallToolResult::success(content)
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
            save: false,
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
            image: None,
            save: None,
        };
        let defaults = sample_defaults();
        let resolved = params.resolve(&defaults);

        assert_eq!(resolved.model, "default-model");
        assert_eq!(resolved.n, 1);
        assert_eq!(resolved.size, "1024x1024");
        assert_eq!(resolved.format, Format::Png);
        assert_eq!(resolved.save, false);
        assert_eq!(resolved.prompt, "hello");
    }

    #[test]
    fn resolve_all_overrides() {
        let params = ImageParams {
            prompt: "hello".to_string(),
            model: Some("custom-model".to_string()),
            n: Some(4),
            size: Some("2048x2048".to_string()),
            format: Some(Format::Jpg),
            image: None,
            save: Some(true),
        };
        let defaults = sample_defaults();
        let resolved = params.resolve(&defaults);

        assert_eq!(resolved.model, "custom-model");
        assert_eq!(resolved.n, 4);
        assert_eq!(resolved.size, "2048x2048");
        assert_eq!(resolved.format, Format::Jpg);
        assert_eq!(resolved.save, true);
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
            image: None,
            save: None,
        };
        let defaults = ImageDefaults {
            model: "default-model".to_string(),
            n: 3,
            size: "512x512".to_string(),
            format: Format::Png,
            save: true,
        };
        let resolved = params.resolve(&defaults);

        assert_eq!(resolved.model, "overridden");
        assert_eq!(resolved.n, 3);
        assert_eq!(resolved.size, "512x512");
        assert_eq!(resolved.format, Format::Webp);
        assert_eq!(resolved.save, true);
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
            image: None,
            save: None,
        };
        let defaults = sample_defaults();
        let resolved = params.resolve(&defaults);
        assert_eq!(resolved.prompt, "my prompt");
    }
}
