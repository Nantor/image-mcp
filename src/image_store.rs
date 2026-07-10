use base64::Engine as _;
use std::path::PathBuf;

use crate::config::Format;

#[derive(Debug, thiserror::Error)]
pub enum ImageStoreError {
    #[error("failed to decode base64 image data: {0}")]
    Decode(#[from] base64::DecodeError),
    #[error("failed to create directory {0}: {1}")]
    CreateDir(PathBuf, std::io::Error),
    #[error("failed to write image to {0}: {1}")]
    Write(PathBuf, std::io::Error),
}

/// Decodes a base64 image and writes it to disk under
/// `~/Pictures/image-mcp/` (falling back to the system temp dir if no home
/// directory can be determined), returning the path written to.
pub fn save_image(b64_data: &str, format: Format) -> Result<PathBuf, ImageStoreError> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64_data)?;

    let dir = save_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| ImageStoreError::CreateDir(dir.clone(), e))?;

    let filename = format!("image-mcp-{}.{}", uuid::Uuid::new_v4(), format.as_str());
    let path = dir.join(filename);

    std::fs::write(&path, &bytes).map_err(|e| ImageStoreError::Write(path.clone(), e))?;

    Ok(path)
}

fn save_dir() -> Result<PathBuf, ImageStoreError> {
    if let Some(pictures) = dirs::picture_dir() {
        return Ok(pictures.join("image-mcp"));
    }
    if let Some(home) = dirs::home_dir() {
        return Ok(home.join("image-mcp"));
    }
    Ok(std::env::temp_dir().join("image-mcp"))
}
