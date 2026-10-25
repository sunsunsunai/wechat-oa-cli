mod builder;

pub use builder::TokenManagerBuilder;

use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};

use crate::{
    client::read_response_limited,
    error::{WechatApiError, WechatError},
};

const TOKEN_ENDPOINT: &str = "https://api.weixin.qq.com/cgi-bin/token";
const DEFAULT_REFRESH_BEFORE: Duration = Duration::from_secs(5 * 60);
const MAX_TOKEN_RESPONSE_LENGTH: usize = 64 * 1024;

/// Obtains and caches a WeChat Official Account access token.
///
/// The cache is shared by all requests using this manager. When the token is
/// close to expiring, only one caller refreshes it; other callers wait for that
/// refresh and then reuse the new value.
pub struct TokenManager {
    app_id: String,
    app_secret: String,
    cached: RwLock<Option<CachedToken>>,
    refresh_lock: Mutex<()>,
    endpoint: String,
    refresh_before: Duration,
}

#[derive(Clone)]
struct CachedToken {
    value: String,
    expires_at: Instant,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
    errcode: Option<i64>,
    errmsg: Option<String>,
    #[serde(alias = "rid")]
    request_id: Option<String>,
}

impl TokenManager {
    pub fn builder() -> TokenManagerBuilder {
        TokenManagerBuilder::new()
    }

    /// Creates a token manager using the public WeChat API endpoint.
    pub fn new(app_id: impl Into<String>, app_secret: impl Into<String>) -> Self {
        Self::with_options(
            app_id.into(),
            app_secret.into(),
            TOKEN_ENDPOINT.to_owned(),
            DEFAULT_REFRESH_BEFORE,
        )
    }

    /// Returns a valid token, refreshing it when necessary.
    pub async fn access_token(&self, http: &reqwest::Client) -> Result<String, WechatError> {
        if let Some(token) = self.cached_token().await {
            return Ok(token);
        }

        // Only one task may refresh at a time. Check the cache again after
        // acquiring the lock because another task may have refreshed it while
        // this task was waiting.
        let _refresh_guard = self.refresh_lock.lock().await;

        if let Some(token) = self.cached_token().await {
            return Ok(token);
        }

        let token = self.fetch_token(http).await?;
        let value = token.value.clone();
        *self.cached.write().await = Some(token);

        Ok(value)
    }

    async fn cached_token(&self) -> Option<String> {
        let cached = self.cached.read().await;
        let token = cached.as_ref()?;

        (token.expires_at.saturating_duration_since(Instant::now()) > self.refresh_before)
            .then(|| token.value.clone())
    }

    /// Invalidates the cache only if it still contains the rejected token.
    ///
    /// Comparing the value prevents a concurrent request from deleting a token
    /// that another task has already refreshed.
    pub(crate) async fn invalidate(&self, rejected_token: &str) {
        let mut cached = self.cached.write().await;
        if cached
            .as_ref()
            .is_some_and(|token| token.value == rejected_token)
        {
            *cached = None;
        }
    }

    async fn fetch_token(&self, http: &reqwest::Client) -> Result<CachedToken, WechatError> {
        let mut url = reqwest::Url::parse(&self.endpoint)
            .map_err(|_| WechatError::InvalidTokenResponse("invalid token endpoint"))?;
        url.query_pairs_mut()
            .append_pair("grant_type", "client_credential")
            .append_pair("appid", &self.app_id)
            .append_pair("secret", &self.app_secret);

        let mut response = http
            .get(url)
            .send()
            .await
            .map_err(|source| WechatError::Http {
                endpoint: self.endpoint.clone(),
                source: source.without_url(),
            })?;
        let status = response.status();
        let body =
            read_response_limited(&mut response, &self.endpoint, MAX_TOKEN_RESPONSE_LENGTH).await?;

        if !status.is_success() {
            return Err(WechatError::HttpStatus {
                endpoint: self.endpoint.clone(),
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body[..body.len().min(512)]).into_owned(),
            });
        }

        let response: TokenResponse =
            serde_json::from_slice(&body).map_err(|source| WechatError::Decode {
                endpoint: self.endpoint.clone(),
                source,
            })?;

        if let Some(code) = response.errcode.filter(|code| *code != 0) {
            return Err(WechatApiError {
                code,
                message: response
                    .errmsg
                    .unwrap_or_else(|| "unknown WeChat API error".to_owned()),
                request_id: response.request_id,
            }
            .into());
        }

        let value = response
            .access_token
            .filter(|token| !token.is_empty())
            .ok_or(WechatError::InvalidTokenResponse("missing access_token"))?;
        let expires_in = response.expires_in.filter(|seconds| *seconds > 0).ok_or(
            WechatError::InvalidTokenResponse("missing or invalid expires_in"),
        )?;
        let expires_at = Instant::now()
            .checked_add(Duration::from_secs(expires_in))
            .ok_or(WechatError::InvalidTokenResponse("expires_in is too large"))?;

        Ok(CachedToken { value, expires_at })
    }

    pub(super) fn with_options(
        app_id: String,
        app_secret: String,
        endpoint: String,
        refresh_before: Duration,
    ) -> Self {
        Self {
            app_id,
            app_secret,
            cached: RwLock::new(None),
            refresh_lock: Mutex::new(()),
            endpoint,
            refresh_before,
        }
    }

    pub(crate) fn app_id(&self) -> &str {
        &self.app_id
    }
}

#[cfg(test)]
mod tests;
