use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize, schemars::JsonSchema, PartialEq, Eq)]
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
