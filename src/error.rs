use thiserror::Error;

/// An application-level error returned by the WeChat API.
///
/// The numeric code is intentionally preserved instead of being converted to
/// a closed enum so newly introduced WeChat error codes remain observable.
#[derive(Debug, Error)]
#[error("WeChat API error {code}: {message}")]
pub struct WechatApiError {
    pub code: i64,
    pub message: String,
    pub request_id: Option<String>,
}

impl WechatApiError {
    pub fn is_token_invalid(&self) -> bool {
        matches!(self.code, 40014 | 42001)
    }

    pub fn is_permission_denied(&self) -> bool {
        self.code == 48001
    }

    pub fn is_rate_limited(&self) -> bool {
        self.code == 45009
    }

    pub fn is_retryable(&self) -> bool {
        self.code == -1
    }
}

/// Errors returned by the WeChat Official Account client.
#[derive(Debug, Error)]
pub enum WechatError {
    /// The underlying HTTP client could not be constructed.
    #[error("failed to build WeChat HTTP client: {0}")]
    HttpClientBuild(#[source] reqwest::Error),

    /// The request could not be sent or its response body could not be read.
    #[error("WeChat HTTP request to {endpoint} failed: {source}")]
    Http {
        endpoint: String,
        #[source]
        source: reqwest::Error,
    },

    /// WeChat returned a non-successful HTTP status.
    #[error("WeChat HTTP request to {endpoint} returned status {status}: {body}")]
    HttpStatus {
        endpoint: String,
        status: u16,
        body: String,
    },

    /// WeChat returned a response larger than the client is willing to buffer.
    #[error("WeChat response from {endpoint} exceeded the {limit} byte limit")]
    ResponseTooLarge { endpoint: String, limit: usize },

    /// A request value could not be encoded as JSON.
    #[error("failed to encode WeChat request: {0}")]
    Encode(#[source] serde_json::Error),

    /// A JSON response from WeChat could not be decoded.
    #[error("failed to decode WeChat response from {endpoint}: {source}")]
    Decode {
        endpoint: String,
        #[source]
        source: serde_json::Error,
    },

    /// WeChat returned an application-level error in a successful HTTP response.
    #[error(transparent)]
    Api(#[from] WechatApiError),

    /// WeChat returned a response that does not match the documented schema.
    #[error("invalid WeChat token response: {0}")]
    InvalidTokenResponse(&'static str),

    /// A configured WeChat API endpoint is not a valid URL.
    #[error("invalid WeChat API endpoint: {0}")]
    InvalidApiEndpoint(String),

    #[error("invalid configuration: {0}")]
    InvalidConfig(&'static str),

    /// A caller supplied a value rejected by the local API contract.
    #[error("invalid parameter `{name}`: {message}")]
    InvalidParameter {
        name: &'static str,
        message: &'static str,
    },

    /// Application configuration could not be loaded.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// A JSON configuration file did not match the expected schema.
    #[error("failed to decode configuration file {}: {source}", path.display())]
    ConfigDecode {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// A JSON configuration file could not be read.
    #[error("failed to read configuration file {}: {source}", path.display())]
    ConfigRead {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A credential file is readable by users other than its owner.
    #[error(
        "credential file {} has insecure permissions {mode:#o}; run `chmod 600 {}`",
        path.display(),
        path.display()
    )]
    InsecureConfigPermissions { path: std::path::PathBuf, mode: u32 },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl WechatError {
    pub(crate) fn is_retryable(&self) -> bool {
        match self {
            Self::Api(error) => error.is_retryable(),
            Self::Http { source, .. } => source.is_connect() || source.is_timeout(),
            Self::HttpStatus { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {name}")]
    MissingEnvironmentVariable { name: &'static str },

    #[error("environment variable is not valid UTF-8: {name}")]
    InvalidEnvironmentVariable { name: &'static str },

    #[error("environment variable cannot be empty: {name}")]
    EmptyEnvironmentVariable { name: &'static str },
}
