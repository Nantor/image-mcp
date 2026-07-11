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
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout())
            .build()
            .expect("reqwest client with timeout should build");
        Self {
            http,
            base_url: normalize_base_url(&config.base_url),
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
    /// tool. `images` is one or more decoded input images, each sent as its
    /// own `image[]` part (per OpenAI's edits API, which accepts an array
    /// of input images) — this lets the model compose/reference multiple
    /// images in a single edit.
    ///
    /// Unlike `generate()`, this does *not* send `response_format`: at
    /// least the `gpt-image-1.5` model rejects it on this endpoint with
    /// `Unknown parameter: 'response_format'` (400), even though the same
    /// field is accepted on `/v1/images/generations`. The endpoint returns
    /// `b64_json` data by default regardless.
    pub async fn edit(
        &self,
        params: &ResolvedParams,
        images: Vec<Vec<u8>>,
    ) -> Result<Vec<String>, LiteLlmError> {
        let url = format!("{}/v1/images/edits", self.base_url);

        let mut form = reqwest::multipart::Form::new()
            .text("prompt", params.prompt.clone())
            .text("model", params.model.clone())
            .text("n", params.n.to_string())
            .text("size", params.size.clone())
            .text("output_format", params.format.as_str());

        for (idx, image_bytes) in images.into_iter().enumerate() {
            let (extension, mime_type) = sniff_image_type(&image_bytes);
            let image_part = reqwest::multipart::Part::bytes(image_bytes)
                .file_name(format!("image-{idx}.{extension}"))
                .mime_str(mime_type)
                .expect("sniffed mime type is valid");
            form = form.part("image[]", image_part);
        }

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

pub(crate) fn normalize_base_url(raw: &str) -> String {
    let stripped = raw.trim_end_matches('/');
    if stripped.ends_with("/v1") {
        stripped.strip_suffix("/v1").unwrap_or(stripped).to_string()
    } else {
        stripped.to_string()
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
    fn strips_trailing_slash() {
        assert_eq!(
            normalize_base_url("http://localhost:4000/"),
            "http://localhost:4000"
        );
    }

    #[test]
    fn keeps_url_without_trailing_slash() {
        assert_eq!(
            normalize_base_url("http://localhost:4000"),
            "http://localhost:4000"
        );
    }

    #[test]
    fn strips_multiple_trailing_slashes() {
        assert_eq!(
            normalize_base_url("http://localhost:4000///"),
            "http://localhost:4000"
        );
    }

    #[test]
    fn strips_trailing_v1() {
        assert_eq!(
            normalize_base_url("http://localhost:4000/v1"),
            "http://localhost:4000"
        );
    }

    #[test]
    fn keeps_v1_in_path() {
        assert_eq!(
            normalize_base_url("http://localhost:4000/some/v1/path"),
            "http://localhost:4000/some/v1/path"
        );
    }

    #[test]
    fn strips_trailing_slash_and_v1() {
        assert_eq!(
            normalize_base_url("http://localhost:4000/v1/"),
            "http://localhost:4000"
        );
    }

    #[test]
    fn handles_https() {
        assert_eq!(
            normalize_base_url("https://adesso-ai-hub.3asabc.de/v1"),
            "https://adesso-ai-hub.3asabc.de"
        );
    }

    #[test]
    fn clones_api_key() {
        let config = LiteLlmConfig {
            base_url: "http://localhost:4000".to_string(),
            api_key: "super-secret-key".to_string(),
            request_timeout_secs: None,
        };
        let client = LiteLlmClient::new(&config);
        assert_eq!(client.api_key, "super-secret-key");
    }

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

#[cfg(test)]
mod integration_tests {
    use wiremock::matchers::{bearer_token, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Format;

    fn sample_params() -> ResolvedParams {
        ResolvedParams {
            prompt: "a red bicycle".to_string(),
            model: "gpt-image-1".to_string(),
            n: 1,
            size: "1024x1024".to_string(),
            format: Format::Png,
            save: false,
        }
    }

    fn client_for(mock_server: &MockServer) -> LiteLlmClient {
        let config = LiteLlmConfig {
            base_url: mock_server.uri(),
            api_key: "test-api-key".to_string(),
            request_timeout_secs: None,
        };
        LiteLlmClient::new(&config)
    }

    #[tokio::test]
    async fn generate_returns_decoded_images_on_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(bearer_token("test-api-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "b64_json": "aGVsbG8=" }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let images = client.generate(&sample_params()).await.unwrap();

        assert_eq!(images, vec!["aGVsbG8=".to_string()]);
    }

    #[tokio::test]
    async fn generate_sends_expected_json_body() {
        use wiremock::matchers::body_json;

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(body_json(json!({
                "prompt": "a red bicycle",
                "model": "gpt-image-1",
                "n": 1,
                "size": "1024x1024",
                "output_format": "png",
                "response_format": "b64_json",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "b64_json": "aGVsbG8=" }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        client.generate(&sample_params()).await.unwrap();
    }

    #[tokio::test]
    async fn generate_returns_multiple_images() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "b64_json": "aW1hZ2Ux" }, { "b64_json": "aW1hZ2Uy" }],
            })))
            .mount(&mock_server)
            .await;

        let mut params = sample_params();
        params.n = 2;
        let client = client_for(&mock_server);
        let images = client.generate(&params).await.unwrap();

        assert_eq!(images, vec!["aW1hZ2Ux".to_string(), "aW1hZ2Uy".to_string()]);
    }

    #[tokio::test]
    async fn generate_surfaces_api_error_status_and_body() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(r#"{"error":"Unknown parameter: 'foo'"}"#),
            )
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let err = client.generate(&sample_params()).await.unwrap_err();

        match err {
            LiteLlmError::Api { status, body } => {
                assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
                assert!(body.contains("Unknown parameter"));
            }
            other => panic!("expected LiteLlmError::Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn generate_errors_on_malformed_json_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let err = client.generate(&sample_params()).await.unwrap_err();

        assert!(matches!(err, LiteLlmError::InvalidResponse(_)));
    }

    #[tokio::test]
    async fn generate_errors_on_empty_image_data() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let err = client.generate(&sample_params()).await.unwrap_err();

        assert!(matches!(err, LiteLlmError::EmptyResponse));
    }

    #[tokio::test]
    async fn generate_errors_on_all_null_b64_json() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "b64_json": null }],
            })))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let err = client.generate(&sample_params()).await.unwrap_err();

        assert!(matches!(err, LiteLlmError::EmptyResponse));
    }

    #[tokio::test]
    async fn edit_returns_decoded_images_on_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(bearer_token("test-api-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "b64_json": "ZWRpdGVk" }],
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let images = client
            .edit(&sample_params(), vec![b"\x89PNG\r\n\x1a\nrest".to_vec()])
            .await
            .unwrap();

        assert_eq!(images, vec!["ZWRpdGVk".to_string()]);
    }

    #[tokio::test]
    async fn edit_sends_one_image_part_per_input_image() {
        use wiremock::Request;

        let mock_server = MockServer::start().await;

        // wiremock doesn't parse multipart bodies out of the box, so we
        // inspect the raw request body for the number of `image[]` field
        // markers, which is the observable contract we care about: one
        // part per input image (verified for real against LiteLLM — see
        // scripts/http-capture/captures/edit/20260710T204757Z).
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(move |req: &Request| {
                let body = String::from_utf8_lossy(&req.body);
                let image_part_count = body.matches("name=\"image[]\"").count();
                assert_eq!(
                    image_part_count, 3,
                    "expected 3 image[] parts, body: {body}"
                );
                ResponseTemplate::new(200).set_body_json(json!({
                    "data": [{ "b64_json": "b2s=" }],
                }))
            })
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let images = vec![
            b"\x89PNG\r\n\x1a\nimg-a".to_vec(),
            b"\xff\xd8\xffimg-b".to_vec(),
            b"img-c-no-magic-bytes".to_vec(),
        ];
        client.edit(&sample_params(), images).await.unwrap();
    }

    #[tokio::test]
    async fn edit_does_not_send_response_format_field() {
        use wiremock::Request;

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(move |req: &Request| {
                let body = String::from_utf8_lossy(&req.body);
                assert!(
                    !body.contains("response_format"),
                    "edit() must not send response_format, per PLAN.md; body: {body}"
                );
                ResponseTemplate::new(200).set_body_json(json!({
                    "data": [{ "b64_json": "b2s=" }],
                }))
            })
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        client
            .edit(&sample_params(), vec![b"\x89PNG\r\n\x1a\nrest".to_vec()])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn edit_surfaces_api_error_status_and_body() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(r#"{"error":"Unknown parameter: 'response_format'"}"#),
            )
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let err = client
            .edit(&sample_params(), vec![b"\x89PNG\r\n\x1a\nrest".to_vec()])
            .await
            .unwrap_err();

        match err {
            LiteLlmError::Api { status, body } => {
                assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
                assert!(body.contains("response_format"));
            }
            other => panic!("expected LiteLlmError::Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn request_fails_when_server_unreachable() {
        // Bind a TCP listener to grab a genuinely free local port, then
        // drop it before connecting, so nothing is listening there. Using
        // a MockServer here doesn't work: `drop()` only schedules async
        // shutdown, so the listener can still accept (and 404) for a
        // window after drop.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = LiteLlmConfig {
            base_url: format!("http://127.0.0.1:{port}"),
            api_key: "test-api-key".to_string(),
            request_timeout_secs: None,
        };
        let client = LiteLlmClient::new(&config);

        let result = client.generate(&sample_params()).await;
        let err = result.expect_err("expected request to fail against an unbound port");
        assert!(
            matches!(err, LiteLlmError::Request { .. }),
            "expected LiteLlmError::Request, got: {err:?}"
        );
    }
}
