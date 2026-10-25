use serde::{Deserialize, Deserializer, Serialize, de};

use super::AnalyticsApi;
use crate::error::WechatError;

const ARTICLE_READ_ENDPOINT: &str = "https://api.weixin.qq.com/datacube/getarticleread";

impl AnalyticsApi<'_> {
    /// Gets daily reading metrics for all published articles read on a date.
    ///
    /// WeChat currently only accepts a one-day range, so `begin_date` and
    /// `end_date` must contain the same `YYYY-MM-DD` value.
    pub async fn article_read(
        &self,
        request: GetArticleReadRequest,
    ) -> Result<GetArticleReadResponse, WechatError> {
        if request.begin_date != request.end_date {
            return Err(WechatError::InvalidConfig(
                "article read only supports a one-day range",
            ));
        }

        self.client
            .post_idempotent(ARTICLE_READ_ENDPOINT, &request)
            .await
    }
}

/// Request for the `getarticleread` API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetArticleReadRequest {
    pub begin_date: String,
    pub end_date: String,
}

impl GetArticleReadRequest {
    /// Creates the one-day range required by WeChat.
    pub fn for_date(date: impl Into<String>) -> Self {
        let date = date.into();
        Self {
            begin_date: date.clone(),
            end_date: date,
        }
    }
}

/// Response returned by the `getarticleread` API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetArticleReadResponse {
    #[serde(default)]
    pub list: Vec<ArticleReadItem>,
    #[serde(deserialize_with = "deserialize_bool_or_string")]
    pub is_delay: bool,
}

/// Reading metrics for one article on one day.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArticleReadItem {
    pub ref_date: String,
    pub msgid: String,
    pub detail: ArticleReadDetail,
}

/// Aggregated reading details for an article.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArticleReadDetail {
    pub read_user: u64,
    #[serde(default)]
    pub read_user_source: Vec<ArticleReadSource>,
}

/// Reading count for one traffic source reported by WeChat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArticleReadSource {
    pub user_count: u64,
    pub scene_desc: String,
}

fn deserialize_bool_or_string<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrString {
        Bool(bool),
        String(String),
    }

    match BoolOrString::deserialize(deserializer)? {
        BoolOrString::Bool(value) => Ok(value),
        BoolOrString::String(value) => value
            .parse::<bool>()
            .map_err(|_| de::Error::custom("expected true, false, \"true\", or \"false\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_one_day_request() {
        let request = GetArticleReadRequest::for_date("2025-11-01");

        assert_eq!(request.begin_date, "2025-11-01");
        assert_eq!(request.end_date, "2025-11-01");
    }

    #[test]
    fn deserializes_documented_response() {
        let response: GetArticleReadResponse = serde_json::from_str(
            r#"{
                "list": [{
                    "ref_date": "2025-11-01",
                    "msgid": "10000050_1",
                    "detail": {
                        "read_user": 4123,
                        "read_user_source": [{
                            "user_count": 234,
                            "scene_desc": "公众号消息"
                        }]
                    }
                }],
                "is_delay": "false"
            }"#,
        )
        .unwrap();

        assert!(!response.is_delay);
        assert_eq!(response.list[0].detail.read_user, 4123);
        assert_eq!(response.list[0].detail.read_user_source[0].user_count, 234);
    }

    #[test]
    fn accepts_boolean_is_delay() {
        let response: GetArticleReadResponse =
            serde_json::from_str(r#"{"list":[],"is_delay":false}"#).unwrap();

        assert!(!response.is_delay);
    }
}
