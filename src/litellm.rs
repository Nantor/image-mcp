use serde::Deserialize;
use serde_json::json;

use crate::config::LiteLlmConfig;
use crate::tools::ResolvedParams;

#[derive(Debug, thiserror::Error)]
pub enum LiteLlmError {
    #[error("failed to reach LiteLLM at {url}: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("LiteLLM returned {status}: {body}")]
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("failed to parse LiteLLM response: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    #[error("LiteLLM response contained no image data")]
    EmptyResponse,
}

#[derive(Debug, Deserialize)]
struct ImagesApiResponse {
    data: Vec<ImageDatum>,
}

#[derive(Debug, Deserialize)]
struct ImageDatum {
    b64_json: Option<String>,
}

pub struct LiteLlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl LiteLlmClient {
    pub fn new(config: &LiteLlmConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
        }
    }

    /// `POST /v1/images/generations` — JSON body, used by the `create` tool.
    pub async fn generate(&self, params: &ResolvedParams) -> Result<Vec<String>, LiteLlmError> {
        let url = format!("{}/v1/images/generations", self.base_url);
        let body = json!({
            "prompt": params.prompt,
            "model": params.model,
            "n": params.n,
            "size": params.size,
            "output_format": params.format.as_str(),
            "response_format": "b64_json",
        });

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|source| LiteLlmError::Request {
                url: url.clone(),
                source,
            })?;

        Self::parse_images_response(response).await
    }

    /// `POST /v1/images/edits` — multipart/form-data body, used by the `edit`
    /// tool. `image_bytes` is the decoded input image.
    pub async fn edit(
        &self,
        params: &ResolvedParams,
        image_bytes: Vec<u8>,
    ) -> Result<Vec<String>, LiteLlmError> {
        let url = format!("{}/v1/images/edits", self.base_url);

        let (extension, mime_type) = sniff_image_type(&image_bytes);
        let image_part = reqwest::multipart::Part::bytes(image_bytes)
            .file_name(format!("image.{extension}"))
            .mime_str(mime_type)
            .expect("sniffed mime type is valid");

        let form = reqwest::multipart::Form::new()
            .text("prompt", params.prompt.clone())
            .text("model", params.model.clone())
            .text("n", params.n.to_string())
            .text("size", params.size.clone())
            .text("output_format", params.format.as_str())
            .text("response_format", "b64_json")
            .part("image[]", image_part);

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|source| LiteLlmError::Request {
                url: url.clone(),
                source,
            })?;

        Self::parse_images_response(response).await
    }

    async fn parse_images_response(
        response: reqwest::Response,
    ) -> Result<Vec<String>, LiteLlmError> {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(LiteLlmError::Api { status, body });
        }

        let parsed: ImagesApiResponse = serde_json::from_str(&body)?;
        let images: Vec<String> = parsed.data.into_iter().filter_map(|d| d.b64_json).collect();

        if images.is_empty() {
            return Err(LiteLlmError::EmptyResponse);
        }

        Ok(images)
    }
}

/// Sniffs an image's format from its magic bytes, defaulting to PNG if
/// unrecognized. Used to set an accurate filename/mime type on the
/// multipart `image[]` part for `/v1/images/edits`.
fn sniff_image_type(bytes: &[u8]) -> (&'static str, &'static str) {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("png", "image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        ("jpg", "image/jpeg")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ("webp", "image/webp")
    } else {
        ("png", "image/png")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_png() {
        let bytes = b"\x89PNG\r\n\x1a\nrest-of-file";
        assert_eq!(sniff_image_type(bytes), ("png", "image/png"));
    }

    #[test]
    fn sniffs_jpeg() {
        let bytes = b"\xff\xd8\xffrest-of-file";
        assert_eq!(sniff_image_type(bytes), ("jpg", "image/jpeg"));
    }

    #[test]
    fn sniffs_webp() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 0]); // chunk size, irrelevant here
        bytes.extend_from_slice(b"WEBP");
        assert_eq!(sniff_image_type(&bytes), ("webp", "image/webp"));
    }

    #[test]
    fn falls_back_to_png_for_unknown_bytes() {
        let bytes = b"totally unrelated data";
        assert_eq!(sniff_image_type(bytes), ("png", "image/png"));
    }
}
