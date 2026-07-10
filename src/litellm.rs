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

        let image_part = reqwest::multipart::Part::bytes(image_bytes)
            .file_name("image.png")
            .mime_str("image/png")
            .expect("static mime type is valid");

        let form = reqwest::multipart::Form::new()
            .text("prompt", params.prompt.clone())
            .text("model", params.model.clone())
            .text("n", params.n.to_string())
            .text("size", params.size.clone())
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
