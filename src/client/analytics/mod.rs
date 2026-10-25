mod article;

pub use article::{
    ArticleReadDetail, ArticleReadItem, ArticleReadSource, GetArticleReadRequest,
    GetArticleReadResponse,
};

use crate::client::WechatOAClient;

/// Entry point for WeChat analytics APIs.
pub struct AnalyticsApi<'a> {
    pub(crate) client: &'a WechatOAClient,
}

impl<'a> AnalyticsApi<'a> {
    pub(crate) fn new(client: &'a WechatOAClient) -> Self {
        Self { client }
    }
}
