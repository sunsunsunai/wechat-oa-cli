mod model;
mod upload;

pub use model::{ArticleImage, PermanentImage, PermanentMediaId};
pub(crate) use upload::{validate_article_image, validate_permanent_image};

use crate::client::WechatOAClient;

/// Entry point for permanent material and article-image uploads.
pub struct MaterialApi<'a> {
    pub(crate) client: &'a WechatOAClient,
}

impl<'a> MaterialApi<'a> {
    pub(crate) fn new(client: &'a WechatOAClient) -> Self {
        Self { client }
    }
}
