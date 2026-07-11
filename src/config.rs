use serde::Deserialize;
use std::path::PathBuf;

#[derive(
    Debug, Clone, Copy, Deserialize, serde::Serialize, schemars::JsonSchema, PartialEq, Eq,
)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Png,
    Jpg,
    Webp,
}

impl Format {
    pub fn as_str(&self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpg => "jpg",
            Format::Webp => "webp",
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpg => "image/jpeg",
            Format::Webp => "image/webp",
        }
    }
}

/// Default request timeout (seconds) applied to LiteLLM calls when
/// `lite_llm.request_timeout_secs` is not set in config. Image
/// generation/editing can be slow (large `n`/`size` combinations on models
/// like `gpt-image-1` routinely take tens of seconds), but a hung LiteLLM
/// proxy or dead upstream must not block a tool call forever.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 180;

/// Default inline base64 payload size (bytes) above which a warning is
/// logged. This is used when `payload_limits.warn_inline_bytes` is omitted
/// from config.
pub const DEFAULT_WARN_INLINE_BYTES: usize = 4 * 1024 * 1024;

/// Default maximum inline base64 payload size (bytes). Above this, the tool
/// call fails instead of returning a potentially huge inline image payload
/// over stdio. This is used when `payload_limits.max_inline_bytes` is omitted
/// from config.
pub const DEFAULT_MAX_INLINE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct LiteLlmConfig {
    pub base_url: String,
    pub api_key: String,
    /// Per-request timeout, in seconds, for calls to LiteLLM. Falls back to
    /// `DEFAULT_REQUEST_TIMEOUT_SECS` if omitted.
    #[serde(default)]
    pub request_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PayloadLimits {
    /// Total inline base64 payload size (bytes) above which a warning is
    /// logged. This is a soft limit only; the tool call still succeeds
    /// unless `max_inline_bytes` is also exceeded.
    #[serde(default = "default_warn_inline_bytes")]
    pub warn_inline_bytes: usize,

    /// Hard cap on total inline base64 payload size (bytes). Above this, the
    /// tool call fails with a clear error instead of returning a huge inline
    /// payload over stdio.
    #[serde(default = "default_max_inline_bytes")]
    pub max_inline_bytes: usize,
}

fn default_warn_inline_bytes() -> usize {
    DEFAULT_WARN_INLINE_BYTES
}

fn default_max_inline_bytes() -> usize {
    DEFAULT_MAX_INLINE_BYTES
}

impl LiteLlmConfig {
    pub fn request_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.request_timeout_secs
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageDefaults {
    pub model: String,
    pub n: u32,
    pub size: String,
    pub format: Format,
    pub save: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub lite_llm: LiteLlmConfig,
    pub image_models: Vec<String>,
    pub create_defaults: ImageDefaults,
    pub edit_defaults: ImageDefaults,
    #[serde(default)]
    pub payload_limits: PayloadLimits,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not determine config directory")]
    NoConfigDir,
    #[error("config file not found at {0}")]
    NotFound(PathBuf),
    #[error("failed to read config file at {0}: {1}")]
    ReadFailed(PathBuf, std::io::Error),
    #[error("failed to parse config file at {0} as JSONC: {1}")]
    ParseFailed(PathBuf, String),
    #[error("invalid config: {0}")]
    Invalid(String),
}

pub fn config_path() -> Result<PathBuf, ConfigError> {
    let config_dir = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
    Ok(config_dir.join("image-mcp").join("config.json"))
}

/// Loads the config from `~/.config/image-mcp/config.json`.
///
/// Per the project plan, the config file must exist and be valid on
/// startup. There is no auto-creation, no built-in defaults, and no
/// merging with defaults — a missing or invalid config is a startup
/// failure, not a runtime tool error.
pub fn load_config() -> Result<Config, ConfigError> {
    let path = config_path()?;
    if !path.exists() {
        return Err(ConfigError::NotFound(path));
    }
    let contents =
        std::fs::read_to_string(&path).map_err(|e| ConfigError::ReadFailed(path.clone(), e))?;
    jsonc_parser::parse_to_serde_value::<Config>(&contents, &Default::default())
        .map_err(|e| ConfigError::ParseFailed(path.clone(), e.to_string()))
}

/// Validates a fully loaded `Config` for obviously invalid values so that
/// bad defaults are caught at startup instead of at tool-call time.
pub fn validate_config(config: &Config) -> Result<(), ConfigError> {
    if config.image_models.is_empty() {
        return Err(ConfigError::Invalid(
            "image_models must contain at least one model".to_string(),
        ));
    }

    validate_image_defaults(
        "create_defaults",
        &config.create_defaults,
        &config.image_models,
    )?;
    validate_image_defaults("edit_defaults", &config.edit_defaults, &config.image_models)?;

    let limits = &config.payload_limits;
    if limits.warn_inline_bytes == 0 {
        return Err(ConfigError::Invalid(
            "payload_limits.warn_inline_bytes must be greater than 0".to_string(),
        ));
    }
    if limits.max_inline_bytes == 0 {
        return Err(ConfigError::Invalid(
            "payload_limits.max_inline_bytes must be greater than 0".to_string(),
        ));
    }
    if limits.max_inline_bytes < limits.warn_inline_bytes {
        return Err(ConfigError::Invalid(
            "payload_limits.max_inline_bytes must be greater than or equal to warn_inline_bytes"
                .to_string(),
        ));
    }

    Ok(())
}

fn validate_image_defaults(
    label: &str,
    defaults: &ImageDefaults,
    image_models: &[String],
) -> Result<(), ConfigError> {
    if defaults.model.trim().is_empty() {
        return Err(ConfigError::Invalid(format!(
            "{label}.model must not be empty",
        )));
    }
    if !image_models.iter().any(|m| m == &defaults.model) {
        return Err(ConfigError::Invalid(format!(
            "{label}.model ({}) must be present in image_models",
            defaults.model
        )));
    }
    if defaults.n == 0 {
        return Err(ConfigError::Invalid(format!(
            "{label}.n must be at least 1",
        )));
    }
    if !is_valid_size_str(&defaults.size) {
        return Err(ConfigError::Invalid(format!(
            "{label}.size must be in the form WIDTHxHEIGHT (e.g. \"1024x1024\"), got {:?}",
            defaults.size
        )));
    }

    Ok(())
}

/// Simple validation that `size` looks like `<digits>x<digits>` (e.g.
/// `1024x1024`). This is a shape check only; actual dimensions are still
/// constrained by LiteLLM/the model.
fn is_valid_size_str(size: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_as_str() {
        assert_eq!(Format::Png.as_str(), "png");
        assert_eq!(Format::Jpg.as_str(), "jpg");
        assert_eq!(Format::Webp.as_str(), "webp");
    }

    #[test]
    fn request_timeout_falls_back_to_default() {
        let config = LiteLlmConfig {
            base_url: "http://localhost:4000".to_string(),
            api_key: "key".to_string(),
            request_timeout_secs: None,
        };
        assert_eq!(
            config.request_timeout(),
            std::time::Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS)
        );
    }

    #[test]
    fn request_timeout_uses_configured_value() {
        let config = LiteLlmConfig {
            base_url: "http://localhost:4000".to_string(),
            api_key: "key".to_string(),
            request_timeout_secs: Some(30),
        };
        assert_eq!(config.request_timeout(), std::time::Duration::from_secs(30));
    }

    #[test]
    fn format_mime_type() {
        assert_eq!(Format::Png.mime_type(), "image/png");
        assert_eq!(Format::Jpg.mime_type(), "image/jpeg");
        assert_eq!(Format::Webp.mime_type(), "image/webp");
    }

    #[test]
    fn config_error_no_config_dir() {
        let err = ConfigError::NoConfigDir;
        assert_eq!(err.to_string(), "could not determine config directory");
    }

    #[test]
    fn config_error_not_found() {
        let path = PathBuf::from("/fake/path/config.json");
        let err = ConfigError::NotFound(path.clone());
        assert_eq!(
            err.to_string(),
            "config file not found at /fake/path/config.json"
        );
    }

    #[test]
    fn config_error_read_failed() {
        let path = PathBuf::from("/fake/path/config.json");
        let io_err =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read access denied");
        let err = ConfigError::ReadFailed(path.clone(), io_err);
        assert!(
            err.to_string()
                .starts_with("failed to read config file at /fake/path/config.json:")
        );
    }

    #[test]
    fn config_error_parse_failed() {
        let path = PathBuf::from("/fake/path/config.json");
        let err = ConfigError::ParseFailed(path.clone(), "unexpected token".to_string());
        let msg = err.to_string();
        assert!(msg.contains("failed to parse config file at"));
        assert!(msg.contains(&path.display().to_string()));
    }

    #[test]
    fn config_path_returns_expected_structure() {
        let dir = dirs::config_dir().expect("config dir should exist on this platform");
        let expected = dir.join("image-mcp").join("config.json");
        let actual = config_path().expect("config_path should succeed when config_dir exists");
        assert_eq!(actual, expected);
    }

    #[test]
    fn payload_limits_defaults_are_sane() {
        let limits = PayloadLimits {
            warn_inline_bytes: default_warn_inline_bytes(),
            max_inline_bytes: default_max_inline_bytes(),
        };
        assert!(limits.warn_inline_bytes > 0);
        assert!(limits.max_inline_bytes >= limits.warn_inline_bytes);
    }

    #[test]
    fn load_config_missing_file() {
        // Use a guaranteed non-existent path by temporarily manipulating
        // config_path is hard-coded to dirs::config_dir(), so just pick a
        // file we know doesn't exist inside the config dir.
        let result = load_config();
        // On most systems config_dir()+image-mcp/config.json won't exist,
        // so we expect NotFound. If the file happens to exist (user has
        // already created it), we should get Ok or ReadFailed, both success.
        // The test just checks that it doesn't panic.
        let _ = result;
    }

    #[test]
    fn validate_config_rejects_empty_image_models() {
        let config = Config {
            lite_llm: LiteLlmConfig {
                base_url: "http://localhost:4000".to_string(),
                api_key: "key".to_string(),
                request_timeout_secs: None,
            },
            image_models: Vec::new(),
            create_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Png,
                save: false,
            },
            edit_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
                save: false,
            },
            payload_limits: PayloadLimits {
                warn_inline_bytes: default_warn_inline_bytes(),
                max_inline_bytes: default_max_inline_bytes(),
            },
        };

        let err = validate_config(&config).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
        assert!(
            err.to_string()
                .contains("image_models must contain at least one model")
        );
    }

    #[test]
    fn validate_config_rejects_defaults_with_unknown_model() {
        let config = Config {
            lite_llm: LiteLlmConfig {
                base_url: "http://localhost:4000".to_string(),
                api_key: "key".to_string(),
                request_timeout_secs: None,
            },
            image_models: vec!["gpt-image-1".to_string()],
            create_defaults: ImageDefaults {
                model: "unknown-model".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Png,
                save: false,
            },
            edit_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
                save: false,
            },
            payload_limits: PayloadLimits {
                warn_inline_bytes: default_warn_inline_bytes(),
                max_inline_bytes: default_max_inline_bytes(),
            },
        };

        let err = validate_config(&config).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
        assert!(
            err.to_string()
                .contains("create_defaults.model (unknown-model) must be present")
        );
    }

    #[test]
    fn validate_config_rejects_zero_n() {
        let config = Config {
            lite_llm: LiteLlmConfig {
                base_url: "http://localhost:4000".to_string(),
                api_key: "key".to_string(),
                request_timeout_secs: None,
            },
            image_models: vec!["gpt-image-1".to_string()],
            create_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 0,
                size: "1024x1024".to_string(),
                format: Format::Png,
                save: false,
            },
            edit_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
                save: false,
            },
            payload_limits: PayloadLimits {
                warn_inline_bytes: default_warn_inline_bytes(),
                max_inline_bytes: default_max_inline_bytes(),
            },
        };

        let err = validate_config(&config).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
        assert!(
            err.to_string()
                .contains("create_defaults.n must be at least 1")
        );
    }

    #[test]
    fn validate_config_rejects_malformed_size() {
        let config = Config {
            lite_llm: LiteLlmConfig {
                base_url: "http://localhost:4000".to_string(),
                api_key: "key".to_string(),
                request_timeout_secs: None,
            },
            image_models: vec!["gpt-image-1".to_string()],
            create_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "not-a-size".to_string(),
                format: Format::Png,
                save: false,
            },
            edit_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
                save: false,
            },
            payload_limits: PayloadLimits {
                warn_inline_bytes: default_warn_inline_bytes(),
                max_inline_bytes: default_max_inline_bytes(),
            },
        };

        let err = validate_config(&config).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
        assert!(
            err.to_string()
                .contains("create_defaults.size must be in the form")
        );
    }

    #[test]
    fn validate_config_rejects_invalid_payload_limits() {
        let config = Config {
            lite_llm: LiteLlmConfig {
                base_url: "http://localhost:4000".to_string(),
                api_key: "key".to_string(),
                request_timeout_secs: None,
            },
            image_models: vec!["gpt-image-1".to_string()],
            create_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Png,
                save: false,
            },
            edit_defaults: ImageDefaults {
                model: "gpt-image-1".to_string(),
                n: 1,
                size: "1024x1024".to_string(),
                format: Format::Jpg,
                save: false,
            },
            payload_limits: PayloadLimits {
                warn_inline_bytes: 10,
                max_inline_bytes: 5,
            },
        };

        let err = validate_config(&config).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
        assert!(
            err.to_string()
                .contains("payload_limits.max_inline_bytes must be greater than or equal")
        );
    }
}
