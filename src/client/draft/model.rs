use serde::{Deserialize, Serialize};

/// One draft returned by the batch-get draft API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftItem {
    pub media_id: String,
    /// Omitted by WeChat when the request contains `no_content: 1`.
    pub content: Option<DraftContent>,
    pub update_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftContent {
    #[serde(default)]
    pub news_item: Vec<DraftArticle>,
}

/// An article inside a draft.
///
/// `article_type` is currently `news` or `newspic`. Fields that only apply to
/// one article type are optional so both response shapes can be represented.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftArticle {
    pub article_type: String,
    pub title: String,
    pub author: Option<String>,
    pub digest: Option<String>,
    pub content: String,
    pub content_source_url: Option<String>,
    pub thumb_media_id: String,
    /// Whether the cover is shown in the article body. Older or image drafts
    /// may omit this field.
    pub show_cover_pic: Option<u8>,
    pub need_open_comment: u8,
    pub only_fans_can_comment: u8,
    pub image_info: Option<ImageInfo>,
    pub cover_info: Option<CoverInfo>,
    pub product_info: Option<ProductInfo>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageInfo {
    #[serde(default)]
    pub image_list: Vec<ImageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageItem {
    pub image_media_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverInfo {
    #[serde(default)]
    pub crop_percent_list: Vec<CoverCrop>,
}

/// Coordinates are strings in the WeChat response schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverCrop {
    pub ratio: String,
    pub x1: String,
    pub y1: String,
    pub x2: String,
    pub y2: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductInfo {
    pub footer_product_info: Option<FooterProductInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FooterProductInfo {
    pub product_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_news_and_newspic_articles() {
        let item: DraftItem = serde_json::from_value(serde_json::json!({
            "media_id": "draft-media-id",
            "content": {
                "news_item": [
                    {
                        "article_type": "news",
                        "title": "Normal article",
                        "author": "Author",
                        "digest": "Digest",
                        "content": "<p>Body</p>",
                        "content_source_url": "https://example.com/source",
                        "thumb_media_id": "thumb-1",
                        "show_cover_pic": 0,
                        "need_open_comment": 0,
                        "only_fans_can_comment": 0,
                        "cover_info": {
                            "crop_percent_list": [{
                                "ratio": "1_1",
                                "x1": "0",
                                "y1": "0",
                                "x2": "1",
                                "y2": "1"
                            }]
                        },
                        "url": "https://mp.weixin.qq.com/draft/1"
                    },
                    {
                        "article_type": "newspic",
                        "title": "Picture article",
                        "content": "Picture text",
                        "thumb_media_id": "thumb-2",
                        "need_open_comment": 1,
                        "only_fans_can_comment": 1,
                        "image_info": {
                            "image_list": [{ "image_media_id": "image-1" }]
                        },
                        "product_info": {
                            "footer_product_info": {
                                "product_key": "product-1"
                            }
                        },
                        "url": "https://mp.weixin.qq.com/draft/2"
                    }
                ]
            },
            "update_time": 1_700_000_000
        }))
        .unwrap();

        let articles = &item.content.unwrap().news_item;
        assert_eq!(articles[0].article_type, "news");
        assert_eq!(articles[0].show_cover_pic, Some(0));
        assert_eq!(
            articles[0]
                .cover_info
                .as_ref()
                .unwrap()
                .crop_percent_list
                .len(),
            1
        );
        assert_eq!(articles[1].article_type, "newspic");
        assert_eq!(articles[1].show_cover_pic, None);
        assert_eq!(articles[1].image_info.as_ref().unwrap().image_list.len(), 1);
        assert_eq!(
            articles[1]
                .product_info
                .as_ref()
                .unwrap()
                .footer_product_info
                .as_ref()
                .unwrap()
                .product_key,
            "product-1"
        );
    }
}
