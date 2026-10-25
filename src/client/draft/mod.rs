mod add;
mod count;
mod get;
mod input;
mod list;
mod model;

pub use add::{AddDraftRequest, AddDraftResponse};
pub use input::{DraftArticleInput, NewsArticleInput, PictureArticleInput};
pub use list::{ListDraftRequest, ListDraftResponse};
pub use model::{
    CoverCrop, CoverInfo, DraftArticle, DraftContent, DraftItem, FooterProductInfo, ImageInfo,
    ImageItem, ProductInfo,
};

use crate::client::WechatOAClient;

pub struct DraftApi<'a> {
    pub(crate) client: &'a WechatOAClient,
}

impl<'a> DraftApi<'a> {
    pub(crate) fn new(client: &'a WechatOAClient) -> Self {
        Self { client }
    }
}
