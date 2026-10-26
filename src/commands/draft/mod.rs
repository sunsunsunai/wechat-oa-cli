mod document;
mod html;
mod state;
mod workflow;

use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{self, AsyncReadExt};

use crate::{
    cli::CliError,
    client::{
        WechatOAClient,
        draft::{
            AddDraftRequest, CoverInfo, DraftArticleInput, ListDraftRequest, NewsArticleInput,
            PictureArticleInput, ProductInfo,
        },
        material::PermanentMediaId,
    },
};

pub use document::{schema, validate};
pub use workflow::push;

use self::{
    state::{DraftState, bytes_sha256},
    workflow::is_unambiguous_failure,
};

const MAX_JSON_INPUT_LENGTH: usize = 8 * 1024 * 1024;

pub async fn count(client: &WechatOAClient) -> Result<Value, CliError> {
    let total_count = client.draft().count().await?;
    Ok(json!({ "total_count": total_count }))
}

pub async fn list(
    client: &WechatOAClient,
    offset: u64,
    count: u8,
    include_content: bool,
) -> Result<Value, CliError> {
    let mut request = ListDraftRequest::new(offset, count)?;
    if !include_content {
        request = request.without_content();
    }
    Ok(serde_json::to_value(client.draft().list(&request).await?)?)
}

pub async fn get(client: &WechatOAClient, media_id: &str) -> Result<Value, CliError> {
    Ok(serde_json::to_value(client.draft().get(media_id).await?)?)
}

pub async fn add_from_input(
    client: &WechatOAClient,
    input: &Path,
    retry_ambiguous: bool,
) -> Result<Value, CliError> {
    let (source_name, bytes) = read_json_input(input).await?;
    let document: AddDraftDocument =
        serde_json::from_slice(&bytes).map_err(|source| CliError::DecodeInput {
            source_name,
            source,
        })?;
    let request = document.into_request()?;
    let fingerprint = bytes_sha256(&serde_json::to_vec(&request)?);
    let (_state_lock, mut state) =
        DraftState::acquire_operation("api-draft-add", client.account_id()).await?;

    if state.has_pending() && !retry_ambiguous {
        return Err(CliError::PendingDraft);
    }

    state.begin_push(fingerprint.clone());
    state.save().await?;
    match client.draft().add(&request).await {
        Ok(response) => {
            state.clear_pending();
            let mut result = serde_json::to_value(response)?;
            if let Err(error) = state.save().await {
                result["state_saved"] = Value::Bool(false);
                result["warning"] = Value::String(format!(
                    "draft was created remotely, but local state could not be saved: {error}"
                ));
            }
            Ok(result)
        }
        Err(source) if is_unambiguous_failure(&source) => {
            state.clear_pending();
            state.save().await?;
            Err(source.into())
        }
        Err(source) => Err(CliError::AmbiguousDraftPush { source }),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddDraftDocument {
    articles: Vec<AddDraftArticle>,
}

impl AddDraftDocument {
    fn into_request(self) -> Result<AddDraftRequest, CliError> {
        let articles = self
            .articles
            .into_iter()
            .map(AddDraftArticle::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(AddDraftRequest::new(articles)?)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "article_type", deny_unknown_fields)]
enum AddDraftArticle {
    #[serde(rename = "news")]
    News {
        title: String,
        content: String,
        thumb_media_id: String,
        #[serde(default)]
        author: Option<String>,
        #[serde(default)]
        digest: Option<String>,
        #[serde(default)]
        content_source_url: Option<String>,
        #[serde(default)]
        need_open_comment: bool,
        #[serde(default)]
        only_fans_can_comment: bool,
        #[serde(default)]
        cover_info: Option<CoverInfo>,
        #[serde(default)]
        product_info: Option<ProductInfo>,
    },
    #[serde(rename = "newspic")]
    Picture {
        title: String,
        content: String,
        image_media_ids: Vec<String>,
        #[serde(default)]
        need_open_comment: bool,
        #[serde(default)]
        only_fans_can_comment: bool,
        #[serde(default)]
        cover_info: Option<CoverInfo>,
        #[serde(default)]
        product_info: Option<ProductInfo>,
    },
}

impl AddDraftArticle {
    fn into_domain(self) -> Result<DraftArticleInput, CliError> {
        match self {
            Self::News {
                title,
                content,
                thumb_media_id,
                author,
                digest,
                content_source_url,
                need_open_comment,
                only_fans_can_comment,
                cover_info,
                product_info,
            } => {
                let mut article =
                    NewsArticleInput::new(title, content, PermanentMediaId::new(thumb_media_id)?)?
                        .open_comments(need_open_comment)
                        .fans_only_comments(only_fans_can_comment);
                if let Some(author) = author {
                    article = article.author(author)?;
                }
                if let Some(digest) = digest {
                    article = article.digest(digest)?;
                }
                if let Some(url) = content_source_url {
                    article = article.content_source_url(url)?;
                }
                if let Some(cover_info) = cover_info {
                    article = article.cover_info(cover_info);
                }
                if let Some(product_info) = product_info {
                    article = article.product_info(product_info);
                }
                Ok(article.into())
            }
            Self::Picture {
                title,
                content,
                image_media_ids,
                need_open_comment,
                only_fans_can_comment,
                cover_info,
                product_info,
            } => {
                let image_media_ids = image_media_ids
                    .into_iter()
                    .map(PermanentMediaId::new)
                    .collect::<Result<Vec<_>, _>>()?;
                let mut article = PictureArticleInput::new(title, content, image_media_ids)?
                    .open_comments(need_open_comment)
                    .fans_only_comments(only_fans_can_comment);
                if let Some(cover_info) = cover_info {
                    article = article.cover_info(cover_info);
                }
                if let Some(product_info) = product_info {
                    article = article.product_info(product_info);
                }
                Ok(article.into())
            }
        }
    }
}

async fn read_json_input(path: &Path) -> Result<(String, Vec<u8>), CliError> {
    if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        io::stdin()
            .take((MAX_JSON_INPUT_LENGTH + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|source| CliError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        ensure_input_size(bytes.len())?;
        Ok(("stdin".to_owned(), bytes))
    } else {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|source| CliError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        if metadata.len() > MAX_JSON_INPUT_LENGTH as u64 {
            return Err(CliError::InputTooLarge {
                limit: MAX_JSON_INPUT_LENGTH,
            });
        }
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|source| CliError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take((MAX_JSON_INPUT_LENGTH + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|source| CliError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        ensure_input_size(bytes.len())?;
        Ok((path.display().to_string(), bytes))
    }
}

fn ensure_input_size(length: usize) -> Result<(), CliError> {
    if length > MAX_JSON_INPUT_LENGTH {
        return Err(CliError::InputTooLarge {
            limit: MAX_JSON_INPUT_LENGTH,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_level_input_is_validated_through_domain_models() {
        let document: AddDraftDocument = serde_json::from_value(json!({
            "articles": [{
                "article_type": "news",
                "title": "Title",
                "content": "<p>Body</p>",
                "thumb_media_id": "thumb-id"
            }]
        }))
        .unwrap();

        assert!(document.into_request().is_ok());
    }

    #[test]
    fn rejects_oversized_json_input() {
        assert!(matches!(
            ensure_input_size(MAX_JSON_INPUT_LENGTH + 1),
            Err(CliError::InputTooLarge {
                limit: MAX_JSON_INPUT_LENGTH
            })
        ));
    }

    #[test]
    fn rejects_unknown_low_level_article_fields() {
        let error = serde_json::from_value::<AddDraftDocument>(json!({
            "articles": [{
                "article_type": "news",
                "title": "Title",
                "content": "<p>Body</p>",
                "thumb_media_id": "thumb-id",
                "need_open_comments": true
            }]
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }
}
