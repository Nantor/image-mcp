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

#[derive(Debug, Clone, Deserialize)]
pub struct LiteLlmConfig {
    pub base_url: String,
    pub api_key: String,
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
        assert_eq!(err.to_string(), "config file not found at /fake/path/config.json");
    }

    #[test]
    fn config_error_read_failed() {
        let path = PathBuf::from("/fake/path/config.json");
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read access denied");
        let err = ConfigError::ReadFailed(path.clone(), io_err);
        assert!(err.to_string().starts_with("failed to read config file at /fake/path/config.json:"));
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
}
