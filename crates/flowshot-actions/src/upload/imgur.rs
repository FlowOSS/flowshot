//! Imgur API v3 uploader.
//!
//! Endpoint: `POST {base_url}/3/image` with header
//! `Authorization: Client-ID {client_id}`. The image is sent as a
//! multipart form field named `image`. Response JSON: `data.link`
//! (public URL) and `data.deletehash` (revoke token).

use std::time::Duration;

use reqwest::multipart::{Form, Part};
use serde::Deserialize;

use super::{UploadMeta, UploadResult, Uploader};
use crate::error::UploadError;

/// Default Imgur API base URL.
const DEFAULT_BASE_URL: &str = "https://api.imgur.com";

/// HTTP request timeout for upload operations.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Imgur upload provider.
#[derive(Debug, Clone)]
pub struct Imgur {
    client_id: String,
    base_url: String,
    client: reqwest::Client,
}

impl Imgur {
    /// Create a new Imgur uploader with the given client id.
    ///
    /// An empty `client_id` means the uploader is unconfigured; calls
    /// to [`Uploader::upload`] will return
    /// [`UploadError::ConfigurationMissing`].
    #[must_use]
    pub fn new(client_id: impl Into<String>) -> Self {
        Self::with_base_url(client_id, DEFAULT_BASE_URL)
    }

    /// Create a new Imgur uploader with a custom base URL (for tests).
    #[must_use]
    pub fn with_base_url(client_id: impl Into<String>, base_url: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(UPLOAD_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client_id: client_id.into(),
            base_url: base_url.into(),
            client,
        }
    }

    /// Whether this uploader is configured (non-empty client id).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        !self.client_id.is_empty()
    }
}

/// Imgur API response structure.
#[derive(Debug, Deserialize)]
struct ImgurResponse {
    data: ImgurData,
    success: bool,
    status: u16,
}

/// Imgur response data payload.
#[derive(Debug, Deserialize)]
struct ImgurData {
    link: String,
    deletehash: String,
}

#[async_trait::async_trait]
impl Uploader for Imgur {
    async fn upload(&self, bytes: &[u8], meta: &UploadMeta) -> Result<UploadResult, UploadError> {
        if !self.is_configured() {
            return Err(UploadError::ConfigurationMissing);
        }

        let url = format!("{}/3/image", self.base_url);
        let filename = meta.filename.clone();
        let form = Form::new()
            .part("image", Part::bytes(bytes.to_vec()).file_name(filename))
            .text("type", "file");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Client-ID {}", self.client_id))
            .multipart(form)
            .send()
            .await
            .map_err(|e| UploadError::Http {
                status: 0,
                message: e.to_string(),
            })?;

        let status = response.status().as_u16();
        if status == 429 {
            return Err(UploadError::RateLimited);
        }
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(UploadError::Http {
                status,
                message: body,
            });
        }

        let imgur_resp: ImgurResponse = response
            .json()
            .await
            .map_err(|e| UploadError::InvalidResponse(format!("failed to parse JSON: {e}")))?;

        if !imgur_resp.success {
            return Err(UploadError::Http {
                status: imgur_resp.status,
                message: "imgur reported failure".to_owned(),
            });
        }

        Ok(UploadResult {
            url: imgur_resp.data.link,
            delete_hash: imgur_resp.data.deletehash,
        })
    }
}

#[cfg(test)]
mod tests {
    use wiremock::Mock;
    use wiremock::{MockServer, ResponseTemplate, matchers};

    use super::*;

    fn meta() -> UploadMeta {
        UploadMeta {
            filename: "test.png".to_owned(),
        }
    }

    fn imgur_success_json() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "link": "https://i.imgur.com/abc123.png",
                "deletehash": "del_abc123"
            },
            "success": true,
            "status": 200
        })
    }

    #[tokio::test]
    async fn unconfigured_client_id_returns_configuration_missing() {
        let imgur = Imgur::new("");
        let result = imgur.upload(b"fake-image-data", &meta()).await;
        assert!(matches!(result, Err(UploadError::ConfigurationMissing)));
    }

    #[tokio::test]
    async fn stub_200_returns_url_and_deletehash() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/3/image"))
            .respond_with(ResponseTemplate::new(200).set_body_json(imgur_success_json()))
            .mount(&server)
            .await;

        let imgur = Imgur::with_base_url("test-client-id", server.uri());
        let result = imgur.upload(b"fake-image-data", &meta()).await;
        let result = match result {
            Ok(r) => r,
            Err(e) => panic!("upload should succeed, got: {e:?}"),
        };
        assert_eq!(result.url, "https://i.imgur.com/abc123.png");
        assert_eq!(result.delete_hash, "del_abc123");
    }

    #[tokio::test]
    async fn stub_429_returns_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/3/image"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let imgur = Imgur::with_base_url("test-client-id", server.uri());
        let result = imgur.upload(b"fake-image-data", &meta()).await;
        assert!(matches!(result, Err(UploadError::RateLimited)));
    }

    #[tokio::test]
    async fn stub_400_returns_invalid_response_or_http() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/3/image"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
            .mount(&server)
            .await;

        let imgur = Imgur::with_base_url("test-client-id", server.uri());
        let result = imgur.upload(b"fake-image-data", &meta()).await;
        assert!(matches!(result, Err(UploadError::Http { status: 400, .. })));
    }

    #[tokio::test]
    async fn stub_500_returns_http_error() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/3/image"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let imgur = Imgur::with_base_url("test-client-id", server.uri());
        let result = imgur.upload(b"fake-image-data", &meta()).await;
        assert!(matches!(result, Err(UploadError::Http { status: 500, .. })));
    }

    #[tokio::test]
    async fn stub_200_with_invalid_json_returns_invalid_response() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/3/image"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let imgur = Imgur::with_base_url("test-client-id", server.uri());
        let result = imgur.upload(b"fake-image-data", &meta()).await;
        assert!(matches!(result, Err(UploadError::InvalidResponse(_))));
    }

    #[tokio::test]
    async fn auth_header_contains_client_id() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/3/image"))
            .and(matchers::header("Authorization", "Client-ID my-secret-id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(imgur_success_json()))
            .mount(&server)
            .await;

        let imgur = Imgur::with_base_url("my-secret-id", server.uri());
        let result = imgur.upload(b"fake-image-data", &meta()).await;
        assert!(result.is_ok());
    }
}
