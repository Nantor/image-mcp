use base64::Engine as _;
use std::path::{Path, PathBuf};

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

/// Decodes a base64 image and writes it to disk, returning the path written
/// to.
///
/// - `target: None` writes under the default save directory
///   (`~/Pictures/image-mcp/`, falling back to `~/image-mcp/` or the system
///   temp dir).
/// - `target: Some(dir)` where `dir` is an existing directory (or a path
///   ending in a path separator) writes inside that directory with a
///   generated filename.
/// - `target: Some(file)` otherwise is treated as an exact destination
///   file path: parent directories are created as needed, and the
///   `format`'s extension is appended if `file` has none.
pub fn save_image(
    b64_data: &str,
    format: Format,
    target: Option<&Path>,
) -> Result<PathBuf, ImageStoreError> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64_data)?;
    // Basic sanity check: ensure the decoded bytes look like the expected
    // image format before writing, so obviously wrong data doesn't get
    // silently written with a misleading file extension.
    if !matches_expected_format(&bytes, format) {
        return Err(ImageStoreError::InvalidFormat(format.as_str()));
    }
    match target {
        Some(path) if is_directory_target(path) => write_image_to_dir(&bytes, path, format),
        Some(path) => write_image_to_file(&bytes, path, format),
        None => {
            let dir = save_dir()?;
            write_image_to_dir(&bytes, &dir, format)
        }
    }
}

/// True if `path` should be treated as a directory to save inside (rather
/// than an exact destination file path): either it already exists as a
/// directory, or it doesn't exist yet but ends with a path separator.
pub fn is_directory_target(path: &Path) -> bool {
    if path.is_dir() {
        return true;
    }
    if path.exists() {
        return false;
    }
    let raw = path.as_os_str().to_string_lossy();
    raw.ends_with('/') || raw.ends_with(std::path::MAIN_SEPARATOR)
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

/// Writes already-decoded image bytes to the exact path `file`, creating
/// its parent directory if needed. If `file` has no extension, the
/// `format`'s extension is appended.
fn write_image_to_file(
    bytes: &[u8],
    file: &Path,
    format: Format,
) -> Result<PathBuf, ImageStoreError> {
    let path = if file.extension().is_none() {
        file.with_extension(format.as_str())
    } else {
        file.to_path_buf()
    };

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| ImageStoreError::CreateDir(parent.to_path_buf(), e))?;
    }

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
        let result = save_image(&b64, Format::Png, None);
        assert!(matches!(result, Err(ImageStoreError::InvalidFormat("png"))));
    }

    #[test]
    fn save_image_rejects_invalid_base64() {
        let result = save_image("not valid base64!!!", Format::Png, None);
        assert!(matches!(result, Err(ImageStoreError::Decode(_))));
    }

    #[test]
    fn save_image_accepts_valid_png_bytes() {
        // Minimal valid PNG header plus some payload bytes.
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let path = save_image(&b64, Format::Png, None).expect("save_image should succeed for PNG");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        let written = std::fs::read(&path).expect("file should exist");
        assert_eq!(written, bytes);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn save_image_with_directory_target_writes_generated_filename_inside_it() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        // Directory does not exist yet: verify the trailing-separator
        // heuristic still treats it as a directory target.
        let mut target = dir.clone().into_os_string();
        target.push(std::path::MAIN_SEPARATOR.to_string());
        let target = PathBuf::from(target);

        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let path = save_image(&b64, Format::Png, Some(&target))
            .expect("save_image should succeed with directory target");
        assert!(path.starts_with(&dir));
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_image_with_existing_directory_target_writes_generated_filename_inside_it() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");

        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let path = save_image(&b64, Format::Png, Some(&dir))
            .expect("save_image should succeed with existing directory target");
        assert!(path.starts_with(&dir));
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_image_with_file_target_writes_exact_path_and_keeps_extension() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("nested").join("my-image.png");

        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let path = save_image(&b64, Format::Png, Some(&file))
            .expect("save_image should succeed with exact file target");
        assert_eq!(path, file);
        let written = std::fs::read(&path).expect("file should exist");
        assert_eq!(written, bytes);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_image_with_file_target_missing_extension_appends_format_extension() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("my-image");

        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let path = save_image(&b64, Format::Png, Some(&file))
            .expect("save_image should succeed with exact file target");
        assert_eq!(path, dir.join("my-image.png"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_directory_target_detects_existing_directory() {
        let dir = std::env::temp_dir();
        assert!(is_directory_target(&dir));
    }

    #[test]
    fn is_directory_target_detects_trailing_separator_on_nonexistent_path() {
        let path = PathBuf::from("/tmp/definitely-does-not-exist-image-mcp/");
        assert!(is_directory_target(&path));
    }

    #[test]
    fn is_directory_target_rejects_plain_nonexistent_file_path() {
        let path = PathBuf::from("/tmp/definitely-does-not-exist-image-mcp/file.png");
        assert!(!is_directory_target(&path));
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
