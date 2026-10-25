use serde::Serialize;

use super::{CoverInfo, ProductInfo};
use crate::{client::material::PermanentMediaId, error::WechatError};

const MAX_TITLE_CHARS: usize = 32;
const MAX_AUTHOR_CHARS: usize = 16;
const MAX_DIGEST_CHARS: usize = 120;
const MAX_CONTENT_CHARS: usize = 20_000;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;
const MAX_PICTURE_COUNT: usize = 20;

/// An article accepted by the add and update draft APIs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "article_type")]
pub enum DraftArticleInput {
    #[serde(rename = "news")]
    News(NewsArticleInput),
    #[serde(rename = "newspic")]
    Picture(PictureArticleInput),
}

impl From<NewsArticleInput> for DraftArticleInput {
    fn from(article: NewsArticleInput) -> Self {
        Self::News(article)
    }
}

impl From<PictureArticleInput> for DraftArticleInput {
    fn from(article: PictureArticleInput) -> Self {
        Self::Picture(article)
    }
}

/// Input for a regular Official Account article (`article_type = news`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NewsArticleInput {
    title: String,
    content: String,
    thumb_media_id: PermanentMediaId,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_source_url: Option<String>,
    need_open_comment: u8,
    only_fans_can_comment: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover_info: Option<CoverInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    product_info: Option<ProductInfo>,
}

impl NewsArticleInput {
    pub fn new(
        title: impl Into<String>,
        content: impl Into<String>,
        thumb_media_id: PermanentMediaId,
    ) -> Result<Self, WechatError> {
        let title = title.into();
        let content = content.into();
        validate_title(&title)?;
        validate_content(&content)?;

        Ok(Self {
            title,
            content,
            thumb_media_id,
            author: None,
            digest: None,
            content_source_url: None,
            need_open_comment: 0,
            only_fans_can_comment: 0,
            cover_info: None,
            product_info: None,
        })
    }

    pub fn author(mut self, author: impl Into<String>) -> Result<Self, WechatError> {
        let author = author.into();
        validate_max_chars(
            "author",
            &author,
            MAX_AUTHOR_CHARS,
            "must not exceed 16 characters",
        )?;
        self.author = Some(author);
        Ok(self)
    }

    pub fn digest(mut self, digest: impl Into<String>) -> Result<Self, WechatError> {
        let digest = digest.into();
        validate_max_chars(
            "digest",
            &digest,
            MAX_DIGEST_CHARS,
            "must not exceed 120 characters",
        )?;
        self.digest = Some(digest);
        Ok(self)
    }

    pub fn content_source_url(mut self, url: impl Into<String>) -> Result<Self, WechatError> {
        let url = url.into();
        validate_required("content_source_url", &url)?;
        self.content_source_url = Some(url);
        Ok(self)
    }

    pub fn open_comments(mut self, enabled: bool) -> Self {
        self.need_open_comment = u8::from(enabled);
        self
    }

    pub fn fans_only_comments(mut self, enabled: bool) -> Self {
        self.only_fans_can_comment = u8::from(enabled);
        self
    }

    pub fn cover_info(mut self, cover_info: CoverInfo) -> Self {
        self.cover_info = Some(cover_info);
        self
    }

    pub fn product_info(mut self, product_info: ProductInfo) -> Self {
        self.product_info = Some(product_info);
        self
    }
}

/// Input for a picture article (`article_type = newspic`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PictureArticleInput {
    title: String,
    content: String,
    image_info: PictureImageInfoInput,
    need_open_comment: u8,
    only_fans_can_comment: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover_info: Option<CoverInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    product_info: Option<ProductInfo>,
}

impl PictureArticleInput {
    pub fn new<I>(
        title: impl Into<String>,
        content: impl Into<String>,
        image_media_ids: I,
    ) -> Result<Self, WechatError>
    where
        I: IntoIterator<Item = PermanentMediaId>,
    {
        let title = title.into();
        let content = content.into();
        validate_title(&title)?;
        validate_content(&content)?;

        let image_list = image_media_ids
            .into_iter()
            .map(|media_id| PictureImageInput {
                image_media_id: media_id,
            })
            .collect::<Vec<_>>();

        if image_list.is_empty() || image_list.len() > MAX_PICTURE_COUNT {
            return Err(WechatError::InvalidParameter {
                name: "image_media_ids",
                message: "must contain between 1 and 20 images",
            });
        }
        Ok(Self {
            title,
            content,
            image_info: PictureImageInfoInput { image_list },
            need_open_comment: 0,
            only_fans_can_comment: 0,
            cover_info: None,
            product_info: None,
        })
    }

    pub fn open_comments(mut self, enabled: bool) -> Self {
        self.need_open_comment = u8::from(enabled);
        self
    }

    pub fn fans_only_comments(mut self, enabled: bool) -> Self {
        self.only_fans_can_comment = u8::from(enabled);
        self
    }

    pub fn cover_info(mut self, cover_info: CoverInfo) -> Self {
        self.cover_info = Some(cover_info);
        self
    }

    pub fn product_info(mut self, product_info: ProductInfo) -> Self {
        self.product_info = Some(product_info);
        self
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PictureImageInfoInput {
    image_list: Vec<PictureImageInput>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PictureImageInput {
    image_media_id: PermanentMediaId,
}

fn validate_title(title: &str) -> Result<(), WechatError> {
    validate_required("title", title)?;
    validate_max_chars(
        "title",
        title,
        MAX_TITLE_CHARS,
        "must not exceed 32 characters",
    )
}

fn validate_content(content: &str) -> Result<(), WechatError> {
    validate_required("content", content)?;
    if content.chars().count() >= MAX_CONTENT_CHARS {
        return Err(WechatError::InvalidParameter {
            name: "content",
            message: "must contain fewer than 20,000 characters",
        });
    }
    if content.len() >= MAX_CONTENT_BYTES {
        return Err(WechatError::InvalidParameter {
            name: "content",
            message: "must be smaller than 1 MiB",
        });
    }
    Ok(())
}

fn validate_required(name: &'static str, value: &str) -> Result<(), WechatError> {
    if value.trim().is_empty() {
        return Err(WechatError::InvalidParameter {
            name,
            message: "cannot be empty",
        });
    }
    Ok(())
}

fn validate_max_chars(
    name: &'static str,
    value: &str,
    max: usize,
    message: &'static str,
) -> Result<(), WechatError> {
    if value.chars().count() > max {
        return Err(WechatError::InvalidParameter { name, message });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media_id(value: &str) -> PermanentMediaId {
        PermanentMediaId::new(value).unwrap()
    }

    #[test]
    fn serializes_news_with_its_article_type() {
        let article = NewsArticleInput::new("标题", "<p>正文</p>", media_id("thumb-id"))
            .unwrap()
            .author("作者")
            .unwrap()
            .open_comments(true);

        assert_eq!(
            serde_json::to_value(DraftArticleInput::from(article)).unwrap(),
            serde_json::json!({
                "article_type": "news",
                "title": "标题",
                "content": "<p>正文</p>",
                "thumb_media_id": "thumb-id",
                "author": "作者",
                "need_open_comment": 1,
                "only_fans_can_comment": 0
            })
        );
    }

    #[test]
    fn serializes_picture_article_with_image_info() {
        let article = PictureArticleInput::new(
            "图片消息",
            "正文",
            [media_id("image-1"), media_id("image-2")],
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(DraftArticleInput::from(article)).unwrap(),
            serde_json::json!({
                "article_type": "newspic",
                "title": "图片消息",
                "content": "正文",
                "image_info": {
                    "image_list": [
                        { "image_media_id": "image-1" },
                        { "image_media_id": "image-2" }
                    ]
                },
                "need_open_comment": 0,
                "only_fans_can_comment": 0
            })
        );
    }

    #[test]
    fn rejects_invalid_required_fields_and_limits() {
        assert!(NewsArticleInput::new("", "body", media_id("thumb")).is_err());
        assert!(NewsArticleInput::new("title", "", media_id("thumb")).is_err());
        assert!(PictureArticleInput::new("title", "body", Vec::<PermanentMediaId>::new()).is_err());
        assert!(PictureArticleInput::new("title", "body", vec![media_id("image"); 21]).is_err());
    }
}
