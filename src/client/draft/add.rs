use serde::{Deserialize, Serialize};

use super::{DraftApi, DraftArticleInput};
use crate::error::WechatError;

const ADD_DRAFT_ENDPOINT: &str = "https://api.weixin.qq.com/cgi-bin/draft/add";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AddDraftRequest {
    articles: Vec<DraftArticleInput>,
}

impl AddDraftRequest {
    pub fn new(articles: Vec<DraftArticleInput>) -> Result<Self, WechatError> {
        if articles.is_empty() {
            return Err(WechatError::InvalidParameter {
                name: "articles",
                message: "must contain at least one article",
            });
        }
        for article in &articles {
            article.validate()?;
        }

        Ok(Self { articles })
    }

    pub fn articles(&self) -> &[DraftArticleInput] {
        &self.articles
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddDraftResponse {
    pub media_id: String,
}

impl DraftApi<'_> {
    /// Adds a draft. Transient failures are not retried because creation is not idempotent.
    pub async fn add(&self, request: &AddDraftRequest) -> Result<AddDraftResponse, WechatError> {
        self.client.post_once(ADD_DRAFT_ENDPOINT, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{draft::NewsArticleInput, material::PermanentMediaId};

    #[test]
    fn serializes_the_request_envelope() {
        let article =
            NewsArticleInput::new("title", "body", PermanentMediaId::new("thumb-id").unwrap())
                .unwrap();
        let request = AddDraftRequest::new(vec![article.into()]).unwrap();

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["articles"][0]["article_type"], "news");
        assert_eq!(value["articles"][0]["thumb_media_id"], "thumb-id");
    }

    #[test]
    fn rejects_an_empty_article_list() {
        assert!(matches!(
            AddDraftRequest::new(Vec::new()),
            Err(WechatError::InvalidParameter {
                name: "articles",
                ..
            })
        ));
    }

    #[test]
    fn rejects_fans_only_comments_when_comments_are_closed() {
        let article =
            NewsArticleInput::new("title", "body", PermanentMediaId::new("thumb-id").unwrap())
                .unwrap()
                .fans_only_comments(true);

        assert!(matches!(
            AddDraftRequest::new(vec![article.into()]),
            Err(WechatError::InvalidParameter {
                name: "only_fans_can_comment",
                ..
            })
        ));
    }

    #[test]
    fn deserializes_the_documented_response() {
        let response: AddDraftResponse =
            serde_json::from_str(r#"{"media_id":"draft-media-id"}"#).unwrap();

        assert_eq!(response.media_id, "draft-media-id");
    }
}
