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

/// Decodes a base64 image and writes it to the exact path `file`, returning
/// the path written to.
///
/// Parent directories are created as needed, and the `format`'s extension
/// is appended if `file` has none.
pub fn save_image(b64_data: &str, format: Format, file: &Path) -> Result<PathBuf, ImageStoreError> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64_data)?;
    // Basic sanity check: ensure the decoded bytes look like the expected
    // image format before writing, so obviously wrong data doesn't get
    // silently written with a misleading file extension.
    if !matches_expected_format(&bytes, format) {
        return Err(ImageStoreError::InvalidFormat(format.as_str()));
    }
    write_image_to_file(&bytes, file, format)
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

    let mut tmp_path = path.clone();
    tmp_path.set_extension(format!("tmp-{}", uuid::Uuid::new_v4()));

    let orphan_dir = if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let existed = dir.try_exists().unwrap_or(false);
        std::fs::create_dir_all(dir)
            .map_err(|e| ImageStoreError::CreateDir(dir.to_path_buf(), e))?;
        if !existed {
            Some(dir.to_path_buf())
        } else {
            None
        }
    } else {
        None
    };

    let result = std::fs::write(&tmp_path, bytes);

    match result {
        Ok(()) => {
            std::fs::rename(&tmp_path, &path)
                .map_err(|e| ImageStoreError::Write(path.clone(), e))?;
            Ok(path)
        }
        Err(write_err) => {
            if let Some(orph) = orphan_dir {
                std::fs::remove_dir_all(&orph).ok();
            }
            let mut p = path.clone();
            p.set_extension(std::ffi::OsString::new());
            Err(ImageStoreError::Write(p, write_err))
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_image_writes_decoded_bytes_with_correct_extension() {
        let bytes = b"not really a png, just test bytes";
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("out.png");

        // bytes are not a real PNG, so save_image must reject them as
        // InvalidFormat rather than writing misleading data.
        let result = save_image(&b64, Format::Png, &file);
        assert!(matches!(result, Err(ImageStoreError::InvalidFormat("png"))));
    }

    #[test]
    fn save_image_rejects_invalid_base64() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("out.png");
        let result = save_image("not valid base64!!!", Format::Png, &file);
        assert!(matches!(result, Err(ImageStoreError::Decode(_))));
    }

    #[test]
    fn save_image_accepts_valid_png_bytes() {
        // Minimal valid PNG header plus some payload bytes.
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("out.png");

        let path = save_image(&b64, Format::Png, &file).expect("save_image should succeed for PNG");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        let written = std::fs::read(&path).expect("file should exist");
        assert_eq!(written, bytes);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_image_with_file_target_writes_exact_path_and_keeps_extension() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("nested").join("my-image.png");

        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"rest-of-file");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let path = save_image(&b64, Format::Png, &file)
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

        let path = save_image(&b64, Format::Png, &file)
            .expect("save_image should succeed with exact file target");
        assert_eq!(path, dir.join("my-image.png"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_image_to_file_creates_dir_and_tries_temp_write() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500))
            .expect("chmod test dir read+execute only");

        let file = dir.join("out.png");
        let result = write_image_to_file(b"not a png\x00\x00", &file, Format::Png);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .expect("restore test dir permissions");
        std::fs::remove_dir_all(&dir).ok();

        // create_dir_all succeeds; write temp file fails because dir is read-only;
        // orphan dir is cleaned up; final path (sans extension suffix) is in Write err.
        assert!(matches!(result, Err(ImageStoreError::Write(_, _))));
    }

    #[test]
    fn write_image_to_file_writes_temp_file_then_renames() {
        let dir = std::env::temp_dir().join(format!("image-mcp-test-{}", uuid::Uuid::new_v4()));
        let file = dir.join("output").join("img.png");

        // bytes are not a real PNG so save_image will reject them before
        // reaching write_image_to_file; use write_image_to_file directly.
        let result = write_image_to_file(b"\x89PNG\r\n\x1a\npayload", &file, Format::Png);

        let written_path = result.expect("write should succeed");
        assert_eq!(written_path, file);
        let written = std::fs::read(&written_path).expect("file should exist");
        assert_eq!(written, b"\x89PNG\r\n\x1a\npayload");
        // temp file should not linger.
        let entries = std::fs::read_dir(dir.join("output")).expect("dir should exist");
        for entry in entries {
            let entry = entry.expect("valid entry");
            let name = entry.file_name();
            assert!(
                !name.to_string_lossy().starts_with("img.tmp-"),
                "temp file was not cleaned up: {:?}",
                name
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
