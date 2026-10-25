use serde::{Deserialize, Serialize};

use super::{DraftApi, DraftItem};
use crate::error::WechatError;

const LIST_DRAFTS_ENDPOINT: &str = "https://api.weixin.qq.com/cgi-bin/draft/batchget";

/// Parameters accepted by the batch-get draft API.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListDraftRequest {
    /// Zero-based offset into all drafts.
    offset: u64,
    /// Number of drafts to return. WeChat accepts values from 1 through 20.
    count: u8,
    /// Set to `1` to omit each item's `content`; omit or set to `0` to include it.
    #[serde(skip_serializing_if = "Option::is_none")]
    no_content: Option<u8>,
}

impl ListDraftRequest {
    pub fn new(offset: u64, count: u8) -> Result<Self, WechatError> {
        if !(1..=20).contains(&count) {
            return Err(WechatError::InvalidParameter {
                name: "count",
                message: "must be between 1 and 20",
            });
        }

        Ok(Self {
            offset,
            count,
            no_content: None,
        })
    }

    pub fn without_content(mut self) -> Self {
        self.no_content = Some(1);
        self
    }
}

/// Response returned by the batch-get draft API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListDraftResponse {
    pub total_count: u64,
    pub item_count: u64,
    #[serde(default)]
    pub item: Vec<DraftItem>,
}

impl DraftApi<'_> {
    pub async fn list(&self, request: &ListDraftRequest) -> Result<ListDraftResponse, WechatError> {
        self.client
            .post_idempotent(LIST_DRAFTS_ENDPOINT, request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_request_with_documented_field_names() {
        let request = ListDraftRequest::new(0, 10).unwrap().without_content();

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "offset": 0,
                "count": 10,
                "no_content": 1
            })
        );
    }

    #[test]
    fn rejects_counts_outside_the_documented_range() {
        assert!(matches!(
            ListDraftRequest::new(0, 0),
            Err(WechatError::InvalidParameter { name: "count", .. })
        ));
        assert!(matches!(
            ListDraftRequest::new(0, 21),
            Err(WechatError::InvalidParameter { name: "count", .. })
        ));
    }

    #[test]
    fn deserializes_items_when_content_is_omitted() {
        let response: ListDraftResponse = serde_json::from_value(serde_json::json!({
            "total_count": 1,
            "item_count": 1,
            "item": [{
                "media_id": "draft-media-id",
                "update_time": 1_700_000_000
            }]
        }))
        .unwrap();

        assert!(response.item[0].content.is_none());
    }

    #[test]
    fn deserializes_article_metadata_when_only_bodies_are_omitted() {
        let response: ListDraftResponse = serde_json::from_value(serde_json::json!({
            "total_count": 1,
            "item_count": 1,
            "item": [{
                "media_id": "draft-media-id",
                "content": {
                    "news_item": [{
                        "article_type": "news",
                        "title": "Metadata only",
                        "thumb_media_id": "thumb-id",
                        "need_open_comment": 0,
                        "only_fans_can_comment": 0,
                        "url": "https://mp.weixin.qq.com/draft/example"
                    }]
                },
                "update_time": 1_700_000_000
            }]
        }))
        .unwrap();

        assert!(
            response.item[0].content.as_ref().unwrap().news_item[0]
                .content
                .is_empty()
        );
    }
}
