use serde::Serialize;

use super::{DraftApi, DraftContent};
use crate::error::WechatError;

const GET_DRAFT_ENDPOINT: &str = "https://api.weixin.qq.com/cgi-bin/draft/get";

#[derive(Debug, Serialize, PartialEq, Eq)]
struct GetDraftRequest<'a> {
    media_id: &'a str,
}

impl<'a> GetDraftRequest<'a> {
    fn new(media_id: &'a str) -> Result<Self, WechatError> {
        if media_id.trim().is_empty() {
            return Err(WechatError::InvalidParameter {
                name: "media_id",
                message: "cannot be empty",
            });
        }

        Ok(Self { media_id })
    }
}

impl DraftApi<'_> {
    /// Gets one draft by its media ID.
    pub async fn get(&self, media_id: &str) -> Result<DraftContent, WechatError> {
        let request = GetDraftRequest::new(media_id)?;
        self.client
            .post_idempotent(GET_DRAFT_ENDPOINT, &request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_the_documented_request() {
        let request = GetDraftRequest::new("draft-media-id").unwrap();

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({ "media_id": "draft-media-id" })
        );
    }

    #[test]
    fn rejects_an_empty_media_id() {
        assert!(matches!(
            GetDraftRequest::new("  "),
            Err(WechatError::InvalidParameter {
                name: "media_id",
                ..
            })
        ));
    }

    #[test]
    fn deserializes_the_documented_response_as_draft_content() {
        let response: DraftContent = serde_json::from_value(serde_json::json!({
            "news_item": [{
                "article_type": "news",
                "title": "Article title",
                "author": "Author",
                "digest": "Digest",
                "content": "<p>Body</p>",
                "content_source_url": "https://example.com/source",
                "thumb_media_id": "thumb-media-id",
                "show_cover_pic": 0,
                "need_open_comment": 0,
                "only_fans_can_comment": 0,
                "url": "https://mp.weixin.qq.com/draft/example"
            }]
        }))
        .unwrap();

        assert_eq!(response.news_item.len(), 1);
        assert_eq!(response.news_item[0].title, "Article title");
        assert_eq!(response.news_item[0].show_cover_pic, Some(0));
    }
}
