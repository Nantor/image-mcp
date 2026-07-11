use base64::Engine as _;
use std::path::PathBuf;

use crate::config::Format;

#[derive(Debug, thiserror::Error)]
pub enum ImageStoreError {
    #[error("failed to decode base64 image data: {0}")]
    Decode(#[from] base64::DecodeError),
    #[error("decoded image bytes did not match expected format {0}")]
    InvalidFormat(&'static str),
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
    // Basic sanity check: ensure the decoded bytes look like the expected
    // image format before writing, so obviously wrong data doesn't get
    // silently written with a misleading file extension.
    if !matches_expected_format(&bytes, format) {
        return Err(ImageStoreError::InvalidFormat(format.as_str()));
    }
    let dir = save_dir()?;
    write_image_to_dir(&bytes, &dir, format)
}

/// Writes already-decoded image bytes under `dir`, creating it if needed.
/// Split out from `save_image` so the create-dir/write error paths can be
/// exercised directly against an arbitrary directory in tests.
fn write_image_to_dir(
    bytes: &[u8],
    dir: &std::path::Path,
    format: Format,
) -> Result<PathBuf, ImageStoreError> {
    std::fs::create_dir_all(dir).map_err(|e| ImageStoreError::CreateDir(dir.to_path_buf(), e))?;

    let filename = format!("image-mcp-{}.{}", uuid::Uuid::new_v4(), format.as_str());
    let path = dir.join(filename);

    std::fs::write(&path, bytes).map_err(|e| ImageStoreError::Write(path.clone(), e))?;

    Ok(path)
}

fn matches_expected_format(bytes: &[u8], format: Format) -> bool {
    if bytes.is_empty() {
        return false;
    }
    match format {
        Format::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        Format::Jpg => bytes.starts_with(b"\xff\xd8\xff"),
        Format::Webp => bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_image_writes_decoded_bytes_with_correct_extension() {
        let bytes = b"not really a png, just test bytes";
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);

        // bytes are not a real PNG, so save_image must reject them as
        // InvalidFormat rather than writing misleading data.
        let result = save_image(&b64, Format::Png);
        assert!(matches!(result, Err(ImageStoreError::InvalidFormat("png"))));
    }

    #[test]
    fn save_image_rejects_invalid_base64() {
        let result = save_image("not valid base64!!!", Format::Png);
        assert!(matches!(result, Err(ImageStoreError::Decode(_))));
    }

    #[test]
    fn save_image_accepts_valid_png_bytes() {
        // Minimal valid PNG header plus some payload bytes.
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let path = save_image(&b64, Format::Png).expect("save_image should succeed for PNG");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        let written = std::fs::read(&path).expect("file should exist");
        assert_eq!(written, bytes);
        std::fs::remove_file(&path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_image_to_dir_fails_when_parent_dir_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).expect("create base test dir");
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o500))
            .expect("chmod base test dir read+execute only");

        let target = base.join("nested");
        let result = write_image_to_dir(b"bytes", &target, Format::Png);

        // Restore writable permissions so the outer tempdir can be cleaned up.
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))
            .expect("restore base test dir permissions");
        std::fs::remove_dir_all(&base).ok();

        match result {
            Err(ImageStoreError::CreateDir(path, _)) => assert_eq!(path, target),
            other => panic!("expected ImageStoreError::CreateDir, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn write_image_to_dir_fails_when_dir_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500))
            .expect("chmod test dir read+execute only");

        let result = write_image_to_dir(b"bytes", &dir, Format::Png);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .expect("restore test dir permissions");
        std::fs::remove_dir_all(&dir).ok();

        assert!(matches!(result, Err(ImageStoreError::Write(_, _))));
    }
}
