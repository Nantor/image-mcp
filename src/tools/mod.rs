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
