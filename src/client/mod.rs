use crate::{
    client::{
        analytics::AnalyticsApi, draft::DraftApi, material::MaterialApi, token::TokenManager,
    },
    error::{WechatApiError, WechatError},
};
use reqwest::{Client as HttpClient, Method};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::Duration;

pub mod analytics;
pub mod draft;
pub mod material;
pub mod publish;
pub mod token;

pub struct WechatOAClient {
    http_client: HttpClient,
    token_manager: TokenManager,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RetryPolicy {
    Never,
    Idempotent,
}

enum RequestBody {
    Json(Vec<u8>),
    Multipart(MultipartUpload),
}

struct MultipartUpload {
    field_name: &'static str,
    file_name: String,
    mime_type: &'static str,
    data: Vec<u8>,
}

const MAX_IDEMPOTENT_ATTEMPTS: u32 = 3;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_ERROR_BODY_LENGTH: usize = 512;
const MAX_RESPONSE_BODY_LENGTH: usize = 16 * 1024 * 1024;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);

impl WechatOAClient {
    pub fn new(token_manager: TokenManager) -> Result<Self, WechatError> {
        let cli = HttpClient::builder()
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .read_timeout(DEFAULT_READ_TIMEOUT)
            .build()
            .map_err(WechatError::HttpClientBuild)?;

        Ok(Self {
            token_manager,
            http_client: cli,
        })
    }

    pub fn builder(token_manager: TokenManager) -> WechatOAClientBuilder {
        WechatOAClientBuilder::new(token_manager)
    }

    pub fn analytics(&self) -> AnalyticsApi<'_> {
        AnalyticsApi::new(self)
    }

    pub fn draft(&self) -> DraftApi<'_> {
        DraftApi::new(self)
    }

    pub fn materials(&self) -> MaterialApi<'_> {
        MaterialApi::new(self)
    }

    /// Verifies the configured credentials without exposing the access token.
    pub async fn check_auth(&self) -> Result<(), WechatError> {
        self.token_manager.access_token(&self.http_client).await?;
        Ok(())
    }

    pub(crate) async fn post_idempotent<B, R>(
        &self,
        endpoint: &str,
        body: &B,
    ) -> Result<R, WechatError>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let request_body = serde_json::to_vec(body).map_err(WechatError::Encode)?;
        self.request(
            Method::POST,
            endpoint,
            Some(RequestBody::Json(request_body)),
            RetryPolicy::Idempotent,
        )
        .await
    }

    pub(crate) async fn post_once<B, R>(&self, endpoint: &str, body: &B) -> Result<R, WechatError>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let request_body = serde_json::to_vec(body).map_err(WechatError::Encode)?;
        self.request(
            Method::POST,
            endpoint,
            Some(RequestBody::Json(request_body)),
            RetryPolicy::Never,
        )
        .await
    }

    pub(crate) async fn post_multipart_once<R>(
        &self,
        endpoint: &str,
        field_name: &'static str,
        file_name: String,
        mime_type: &'static str,
        data: Vec<u8>,
    ) -> Result<R, WechatError>
    where
        R: DeserializeOwned,
    {
        self.request(
            Method::POST,
            endpoint,
            Some(RequestBody::Multipart(MultipartUpload {
                field_name,
                file_name,
                mime_type,
                data,
            })),
            RetryPolicy::Never,
        )
        .await
    }

    /// Sends an idempotent GET request authenticated with the current access token.
    pub(crate) async fn get<R>(&self, endpoint: &str) -> Result<R, WechatError>
    where
        R: DeserializeOwned,
    {
        self.request(Method::GET, endpoint, None, RetryPolicy::Idempotent)
            .await
    }

    async fn request<R>(
        &self,
        method: Method,
        endpoint: &str,
        request_body: Option<RequestBody>,
        retry_policy: RetryPolicy,
    ) -> Result<R, WechatError>
    where
        R: DeserializeOwned,
    {
        let mut access_token = self.token_manager.access_token(&self.http_client).await?;
        let mut token_refreshed = false;
        let mut attempt = 1;

        loop {
            let result = self
                .send_once(&method, endpoint, &access_token, request_body.as_ref())
                .await
                .and_then(|response_body| decode_response(endpoint, &response_body));

            match result {
                Err(WechatError::Api(ref error))
                    if error.is_token_invalid() && !token_refreshed =>
                {
                    access_token = self
                        .token_manager
                        .refresh_rejected(&self.http_client, &access_token)
                        .await?;
                    token_refreshed = true;
                }
                Err(ref error)
                    if retry_policy == RetryPolicy::Idempotent
                        && error.is_retryable()
                        && attempt < MAX_IDEMPOTENT_ATTEMPTS =>
                {
                    let delay = INITIAL_RETRY_DELAY * 2_u32.pow(attempt - 1);
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn send_once(
        &self,
        method: &Method,
        endpoint: &str,
        access_token: &str,
        request_body: Option<&RequestBody>,
    ) -> Result<Vec<u8>, WechatError> {
        let mut url = reqwest::Url::parse(endpoint)
            .map_err(|_| WechatError::InvalidApiEndpoint(endpoint.to_owned()))?;
        url.query_pairs_mut()
            .append_pair("access_token", access_token);

        let mut request = self.http_client.request(method.clone(), url);
        if let Some(request_body) = request_body {
            request = match request_body {
                RequestBody::Json(data) => request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(data.clone()),
                RequestBody::Multipart(upload) => {
                    let part = reqwest::multipart::Part::bytes(upload.data.clone())
                        .file_name(upload.file_name.clone())
                        .mime_str(upload.mime_type)
                        .map_err(|source| WechatError::Http {
                            endpoint: endpoint.to_owned(),
                            source: source.without_url(),
                        })?;
                    let form =
                        reqwest::multipart::Form::new().part(upload.field_name.to_owned(), part);
                    request.multipart(form)
                }
            };
        }

        let mut response = request.send().await.map_err(|source| WechatError::Http {
            endpoint: endpoint.to_owned(),
            source: source.without_url(),
        })?;
        let status = response.status();
        let response_body =
            read_response_limited(&mut response, endpoint, MAX_RESPONSE_BODY_LENGTH).await?;

        if !status.is_success() {
            return Err(WechatError::HttpStatus {
                endpoint: endpoint.to_owned(),
                status: status.as_u16(),
                body: response_excerpt(&response_body),
            });
        }

        Ok(response_body)
    }

    pub(crate) fn account_id(&self) -> &str {
        self.token_manager.app_id()
    }
}

pub(crate) async fn read_response_limited(
    response: &mut reqwest::Response,
    endpoint: &str,
    limit: usize,
) -> Result<Vec<u8>, WechatError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(WechatError::ResponseTooLarge {
            endpoint: endpoint.to_owned(),
            limit,
        });
    }

    let mut body =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(limit as u64) as usize);
    while let Some(chunk) = response.chunk().await.map_err(|source| WechatError::Http {
        endpoint: endpoint.to_owned(),
        source: source.without_url(),
    })? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(WechatError::ResponseTooLarge {
                endpoint: endpoint.to_owned(),
                limit,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct ApiErrorResponse {
    errcode: Option<i64>,
    errmsg: Option<String>,
    #[serde(alias = "rid")]
    request_id: Option<String>,
}

fn decode_response<R: DeserializeOwned>(endpoint: &str, body: &[u8]) -> Result<R, WechatError> {
    let api_error: ApiErrorResponse =
        serde_json::from_slice(body).map_err(|source| WechatError::Decode {
            endpoint: endpoint.to_owned(),
            source,
        })?;

    if let Some(code) = api_error.errcode.filter(|code| *code != 0) {
        return Err(WechatApiError {
            code,
            message: api_error
                .errmsg
                .unwrap_or_else(|| "unknown WeChat API error".to_owned()),
            request_id: api_error.request_id,
        }
        .into());
    }

    serde_json::from_slice(body).map_err(|source| WechatError::Decode {
        endpoint: endpoint.to_owned(),
        source,
    })
}

fn response_excerpt(body: &[u8]) -> String {
    let end = body.len().min(MAX_ERROR_BODY_LENGTH);
    let mut excerpt = String::from_utf8_lossy(&body[..end]).into_owned();
    if body.len() > MAX_ERROR_BODY_LENGTH {
        excerpt.push('…');
    }
    excerpt
}

pub struct WechatOAClientBuilder {
    token_manager: TokenManager,
}

impl WechatOAClientBuilder {
    pub fn new(token_manager: TokenManager) -> Self {
        Self { token_manager }
    }

    pub fn build(self) -> Result<WechatOAClient, WechatError> {
        WechatOAClient::new(self.token_manager)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[derive(Clone, Copy)]
    enum ApiBehavior {
        RejectFirstToken,
        BusyTwice,
        Get,
    }

    async fn test_server(behavior: ApiBehavior) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let token_requests = Arc::new(AtomicUsize::new(0));
        let api_requests = Arc::new(AtomicUsize::new(0));
        let token_count = Arc::clone(&token_requests);
        let api_count = Arc::clone(&api_requests);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = [0_u8; 4096];
                let bytes_read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..bytes_read]);

                let body = if request.starts_with("POST /token") {
                    let call = token_count.fetch_add(1, Ordering::SeqCst);
                    let token = match behavior {
                        ApiBehavior::RejectFirstToken if call == 0 => "token-1",
                        _ => "token-2",
                    };
                    format!(r#"{{"access_token":"{token}","expires_in":7200}}"#)
                } else {
                    let call = api_count.fetch_add(1, Ordering::SeqCst);
                    match behavior {
                        ApiBehavior::RejectFirstToken
                            if request.contains("access_token=token-1") =>
                        {
                            r#"{"errcode":42001,"errmsg":"access token expired"}"#.to_owned()
                        }
                        ApiBehavior::BusyTwice if call < 2 => {
                            r#"{"errcode":-1,"errmsg":"system busy"}"#.to_owned()
                        }
                        ApiBehavior::Get
                            if request.starts_with("GET /drafts?access_token=token-2") =>
                        {
                            r#"{"count":7}"#.to_owned()
                        }
                        ApiBehavior::Get => {
                            r#"{"errcode":40000,"errmsg":"expected authenticated GET"}"#.to_owned()
                        }
                        _ => r#"{"ok":true}"#.to_owned(),
                    }
                };

                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        (format!("http://{address}"), token_requests, api_requests)
    }

    #[test]
    fn preserves_unknown_api_errors_and_request_ids() {
        let error = decode_response::<serde_json::Value>(
            "https://api.weixin.qq.com/test",
            br#"{"errcode":123456,"errmsg":"new error","rid":"request-1"}"#,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            WechatError::Api(WechatApiError {
                code: 123456,
                ref message,
                request_id: Some(ref request_id),
            }) if message == "new error" && request_id == "request-1"
        ));
    }

    #[test]
    fn decode_errors_include_the_endpoint() {
        let error =
            decode_response::<serde_json::Value>("https://api.weixin.qq.com/test", b"not-json")
                .unwrap_err();

        assert!(matches!(
            error,
            WechatError::Decode { ref endpoint, .. }
                if endpoint == "https://api.weixin.qq.com/test"
        ));
    }

    #[tokio::test]
    async fn refreshes_a_rejected_token_once_and_replays_the_request() {
        let (base_url, token_requests, api_requests) =
            test_server(ApiBehavior::RejectFirstToken).await;
        let token_manager = TokenManager::with_options(
            "app-id".to_owned(),
            "secret".to_owned(),
            format!("{base_url}/token"),
            Duration::from_secs(300),
        );
        let client = WechatOAClient::new(token_manager).unwrap();
        let endpoint = format!("{base_url}/drafts");

        let response: serde_json::Value = client
            .post_idempotent(&endpoint, &serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(response, serde_json::json!({ "ok": true }));
        assert_eq!(token_requests.load(Ordering::SeqCst), 2);
        assert_eq!(api_requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retries_transient_api_errors_for_idempotent_requests() {
        let (base_url, token_requests, api_requests) = test_server(ApiBehavior::BusyTwice).await;
        let token_manager = TokenManager::with_options(
            "app-id".to_owned(),
            "secret".to_owned(),
            format!("{base_url}/token"),
            Duration::from_secs(300),
        );
        let client = WechatOAClient::new(token_manager).unwrap();
        let endpoint = format!("{base_url}/drafts");

        let response: serde_json::Value = client
            .post_idempotent(&endpoint, &serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(response, serde_json::json!({ "ok": true }));
        assert_eq!(token_requests.load(Ordering::SeqCst), 1);
        assert_eq!(api_requests.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn post_once_does_not_retry_transient_api_errors() {
        let (base_url, token_requests, api_requests) = test_server(ApiBehavior::BusyTwice).await;
        let token_manager = TokenManager::with_options(
            "app-id".to_owned(),
            "secret".to_owned(),
            format!("{base_url}/token"),
            Duration::from_secs(300),
        );
        let client = WechatOAClient::new(token_manager).unwrap();
        let endpoint = format!("{base_url}/drafts");

        let error = client
            .post_once::<_, serde_json::Value>(&endpoint, &serde_json::json!({}))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            WechatError::Api(WechatApiError { code: -1, .. })
        ));
        assert_eq!(token_requests.load(Ordering::SeqCst), 1);
        assert_eq!(api_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn post_once_refreshes_a_token_explicitly_rejected_by_wechat() {
        let (base_url, token_requests, api_requests) =
            test_server(ApiBehavior::RejectFirstToken).await;
        let token_manager = TokenManager::with_options(
            "app-id".to_owned(),
            "secret".to_owned(),
            format!("{base_url}/token"),
            Duration::from_secs(300),
        );
        let client = WechatOAClient::new(token_manager).unwrap();
        let endpoint = format!("{base_url}/drafts");

        let response: serde_json::Value = client
            .post_once(&endpoint, &serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(response, serde_json::json!({ "ok": true }));
        assert_eq!(token_requests.load(Ordering::SeqCst), 2);
        assert_eq!(api_requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn get_sends_an_authenticated_request_and_decodes_the_response() {
        let (base_url, token_requests, api_requests) = test_server(ApiBehavior::Get).await;
        let token_manager = TokenManager::with_options(
            "app-id".to_owned(),
            "secret".to_owned(),
            format!("{base_url}/token"),
            Duration::from_secs(300),
        );
        let client = WechatOAClient::new(token_manager).unwrap();
        let endpoint = format!("{base_url}/drafts");

        let response: serde_json::Value = client.get(&endpoint).await.unwrap();

        assert_eq!(response, serde_json::json!({ "count": 7 }));
        assert_eq!(token_requests.load(Ordering::SeqCst), 1);
        assert_eq!(api_requests.load(Ordering::SeqCst), 1);
    }
}
