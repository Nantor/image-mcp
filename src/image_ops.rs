use std::io::Cursor;

use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};

use crate::config::Format;

#[derive(Debug, thiserror::Error)]
pub enum ImageOpsError {
    #[error("could not determine image format from file contents")]
    UnrecognizedFormat,
    #[error("unsupported image format {0:?}; only png, jpg, and webp are supported")]
    UnsupportedFormat(ImageFormat),
    #[error("failed to read image dimensions: {0}")]
    Dimensions(image::ImageError),
    #[error("failed to decode image: {0}")]
    Decode(image::ImageError),
    #[error("failed to encode image: {0}")]
    Encode(image::ImageError),
}

pub struct ImageInfo {
    pub format: Format,
    pub width: u32,
    pub height: u32,
}

/// Detects an image's on-disk format from its magic bytes and maps it to
/// our supported `Format` enum (png/jpg/webp). Errors if the format can't
/// be recognized at all, or is recognized but isn't one of the three
/// supported formats.
pub fn detect_format(bytes: &[u8]) -> Result<Format, ImageOpsError> {
    let guessed = image::guess_format(bytes).map_err(|_| ImageOpsError::UnrecognizedFormat)?;
    map_format(guessed)
}

fn map_format(fmt: ImageFormat) -> Result<Format, ImageOpsError> {
    match fmt {
        ImageFormat::Png => Ok(Format::Png),
        ImageFormat::Jpeg => Ok(Format::Jpg),
        ImageFormat::WebP => Ok(Format::Webp),
        other => Err(ImageOpsError::UnsupportedFormat(other)),
    }
}

fn to_image_format(format: Format) -> ImageFormat {
    match format {
        Format::Png => ImageFormat::Png,
        Format::Jpg => ImageFormat::Jpeg,
        Format::Webp => ImageFormat::WebP,
    }
}

/// Reads an image's format and pixel dimensions from raw file bytes,
/// without fully decoding pixel data.
pub fn inspect(bytes: &[u8]) -> Result<ImageInfo, ImageOpsError> {
    let format = detect_format(bytes)?;
    let reader = ImageReader::with_format(Cursor::new(bytes), to_image_format(format));
    let (width, height) = reader
        .into_dimensions()
        .map_err(ImageOpsError::Dimensions)?;
    Ok(ImageInfo {
        format,
        width,
        height,
    })
}

/// Decodes `bytes` (already known to be `input_format`), resizes to
/// exactly `width`x`height` — stretching the image if the aspect ratio
/// differs, never cropping or letterboxing — and re-encodes to
/// `output_format`. Returns the encoded output bytes.
pub fn resize(
    bytes: &[u8],
    input_format: Format,
    width: u32,
    height: u32,
    output_format: Format,
) -> Result<Vec<u8>, ImageOpsError> {
    let decoded = image::load_from_memory_with_format(bytes, to_image_format(input_format))
        .map_err(ImageOpsError::Decode)?;
    let resized = decoded.resize_exact(width, height, FilterType::Lanczos3);

    let mut buf = Cursor::new(Vec::new());
    resized
        .write_to(&mut buf, to_image_format(output_format))
        .map_err(ImageOpsError::Encode)?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let img = image::DynamicImage::new_rgb8(width, height);
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, format).unwrap();
        buf.into_inner()
    }

    #[test]
    fn detect_format_recognizes_png() {
        let bytes = encode(ImageFormat::Png, 4, 4);
        assert_eq!(detect_format(&bytes).unwrap(), Format::Png);
    }

    #[test]
    fn detect_format_recognizes_jpeg() {
        let bytes = encode(ImageFormat::Jpeg, 4, 4);
        assert_eq!(detect_format(&bytes).unwrap(), Format::Jpg);
    }

    #[test]
    fn detect_format_recognizes_webp() {
        let bytes = encode(ImageFormat::WebP, 4, 4);
        assert_eq!(detect_format(&bytes).unwrap(), Format::Webp);
    }

    #[test]
    fn detect_format_rejects_unrecognized_bytes() {
        let bytes = b"not an image at all";
        assert!(matches!(
            detect_format(bytes),
            Err(ImageOpsError::UnrecognizedFormat)
        ));
    }

    #[test]
    fn detect_format_rejects_unsupported_recognized_format() {
        // Minimal BMP magic bytes: recognized by `guess_format` from just
        // the header, but bmp isn't one of our enabled/supported codecs
        // (and doesn't need a full valid body to be recognized).
        let bytes = b"BM\x3a\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00";
        assert!(matches!(
            detect_format(bytes),
            Err(ImageOpsError::UnsupportedFormat(ImageFormat::Bmp))
        ));
    }

    #[test]
    fn inspect_returns_format_and_dimensions() {
        let bytes = encode(ImageFormat::Png, 37, 51);
        let info = inspect(&bytes).unwrap();
        assert_eq!(info.format, Format::Png);
        assert_eq!(info.width, 37);
        assert_eq!(info.height, 51);
    }

    #[test]
    fn inspect_errors_on_garbage_bytes() {
        let bytes = b"totally not an image";
        assert!(matches!(
            inspect(bytes),
            Err(ImageOpsError::UnrecognizedFormat)
        ));
    }

    #[test]
    fn resize_stretches_to_exact_dimensions() {
        let bytes = encode(ImageFormat::Png, 10, 20);
        let resized = resize(&bytes, Format::Png, 40, 5, Format::Png).unwrap();
        let info = inspect(&resized).unwrap();
        assert_eq!(info.width, 40);
        assert_eq!(info.height, 5);
        assert_eq!(info.format, Format::Png);
    }

    #[test]
    fn resize_can_convert_format() {
        let bytes = encode(ImageFormat::Png, 10, 10);
        let resized = resize(&bytes, Format::Png, 5, 5, Format::Jpg).unwrap();
        let info = inspect(&resized).unwrap();
        assert_eq!(info.format, Format::Jpg);
        assert_eq!(info.width, 5);
        assert_eq!(info.height, 5);
    }

    #[test]
    fn resize_errors_on_corrupt_bytes() {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(b"this is not a valid png body");
        assert!(matches!(
            resize(&bytes, Format::Png, 10, 10, Format::Png),
            Err(ImageOpsError::Decode(_))
        ));
    }
}
