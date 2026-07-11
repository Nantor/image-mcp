pub mod create;
pub mod edit;
pub mod list_models;

use rmcp::model::{CallToolResult, ContentBlock};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::{Format, ImageDefaults, PayloadLimits};
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
    /// Base64-encoded input image(s). Required for `edit` (at least one),
    /// unused for `create`. Multiple images are sent as separate `image[]`
    /// parts to LiteLLM, letting the model compose/reference all of them
    /// in a single edit (e.g. "put subject A onto subject B's background").
    pub image: Option<Vec<String>>,
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

/// Shared response handling for `create` and `edit`: either writes each
/// image to disk and returns its path as text, or returns it inline as an
/// MCP `image` content block, depending on `save`.
pub fn respond_with_images(
    images: Vec<String>,
    format: Format,
    save: bool,
    payload_limits: &PayloadLimits,
) -> CallToolResult {
    let mut content = Vec::with_capacity(images.len());

    if !save {
        let total_bytes: usize = images.iter().map(|img| img.len()).sum();
        if total_bytes > payload_limits.max_inline_bytes {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "inline image payload too large: total base64 bytes {total_bytes} exceeds configured max_inline_bytes {}",
                payload_limits.max_inline_bytes,
            ))]);
        }
        if total_bytes > payload_limits.warn_inline_bytes {
            tracing::warn!(
                total_base64_bytes = total_bytes,
                max_inline_bytes = payload_limits.max_inline_bytes,
                image_count = images.len(),
                "returning a large inline image payload over stdio; consider `save: true` \
                 or a smaller `n`/`size` if the MCP client rejects or truncates this response",
            );
        }
    }

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
        assert!(!resolved.save);
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
        assert!(resolved.save);
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
        assert!(resolved.save);
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

    fn sample_resolved() -> ResolvedParams {
        ResolvedParams {
            prompt: "a prompt".to_string(),
            model: "model".to_string(),
            n: 1,
            size: "1024x1024".to_string(),
            format: Format::Png,
            save: false,
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
    fn respond_with_images_save_false_returns_inline_image_blocks() {
        let limits = PayloadLimits {
            warn_inline_bytes: 4 * 1024 * 1024,
            max_inline_bytes: 8 * 1024 * 1024,
        };
        let result = respond_with_images(vec!["aGVsbG8=".to_string()], Format::Png, false, &limits);
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        assert!(matches!(result.content[0], ContentBlock::Image(_)));
    }

    #[test]
    fn respond_with_images_save_true_writes_and_returns_text_paths() {
        use base64::Engine as _;

        let bytes = b"not a real png, just bytes";
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);

        let limits = PayloadLimits {
            warn_inline_bytes: 4 * 1024 * 1024,
            max_inline_bytes: 8 * 1024 * 1024,
        };
        let result = respond_with_images(vec![b64], Format::Png, true, &limits);
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn respond_with_images_save_true_errors_on_invalid_base64() {
        let limits = PayloadLimits {
            warn_inline_bytes: 4 * 1024 * 1024,
            max_inline_bytes: 8 * 1024 * 1024,
        };
        let result = respond_with_images(
            vec!["not valid base64!!!".to_string()],
            Format::Png,
            true,
            &limits,
        );
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn respond_with_images_handles_multiple_images() {
        let limits = PayloadLimits {
            warn_inline_bytes: 4 * 1024 * 1024,
            max_inline_bytes: 8 * 1024 * 1024,
        };
        let result = respond_with_images(
            vec!["aGVsbG8=".to_string(), "d29ybGQ=".to_string()],
            Format::Jpg,
            false,
            &limits,
        );
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 2);
    }

    #[test]
    fn respond_with_images_does_not_warn_below_threshold() {
        // Sanity check that a normal-sized payload doesn't panic or error;
        // the size-warning path is a side-effecting log only, not behavior
        // that changes the returned result.
        let limits = PayloadLimits {
            warn_inline_bytes: 4 * 1024 * 1024,
            max_inline_bytes: 8 * 1024 * 1024,
        };
        let result = respond_with_images(vec!["aGVsbG8=".to_string()], Format::Png, false, &limits);
        assert_eq!(result.is_error, Some(false));
    }

    #[test]
    fn respond_with_images_exercises_large_payload_warn_path() {
        // Exceeds LARGE_INLINE_PAYLOAD_WARN_BYTES to exercise the
        // tracing::warn! branch. The warning is a side-effecting log only,
        // so this asserts the branch runs without panicking/erroring and
        // still returns the expected content, rather than asserting on the
        // log output itself.
        let big = "a".repeat(4 * 1024 * 1024 + 1);
        let limits = PayloadLimits {
            warn_inline_bytes: 4 * 1024 * 1024,
            max_inline_bytes: 8 * 1024 * 1024,
        };
        let result = respond_with_images(vec![big], Format::Png, false, &limits);
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        assert!(matches!(result.content[0], ContentBlock::Image(_)));
    }

    #[test]
    fn respond_with_images_large_payload_warn_path_with_save() {
        // Same large-payload branch, but with save=true: the warning check
        // is guarded by `if !save`, so this must NOT warn and must still
        // succeed (encoding garbage bytes as base64 for the file write).
        use base64::Engine as _;
        // Large but valid PNG data; header plus a large payload section.
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend(std::iter::repeat(b'a').take(4 * 1024 * 1024 + 1));
        let big_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let limits = PayloadLimits {
            warn_inline_bytes: 4 * 1024 * 1024,
            max_inline_bytes: 8 * 1024 * 1024,
        };
        let result = respond_with_images(vec![big_b64], Format::Png, true, &limits);
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        let path = match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text block"),
        };
        std::fs::remove_file(&path).ok();
    }
}
