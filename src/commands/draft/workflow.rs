use std::{collections::HashMap, path::Path, path::PathBuf};

use serde_json::{Value, json};

use super::{
    document::{ResolvedArticle, ResolvedDraft},
    html::{local_image_sources, rewrite_local_images},
    state::{DraftState, LegacyStatePolicy, bytes_sha256, file_sha256},
};
use crate::{
    cli::{CliError, LegacyStateAction},
    client::{
        WechatOAClient,
        draft::AddDraftRequest,
        material::{validate_article_image, validate_permanent_image},
    },
    error::WechatError,
};

pub async fn push(
    client: &WechatOAClient,
    path: &Path,
    retry_ambiguous: bool,
    legacy_state: Option<LegacyStateAction>,
) -> Result<Value, CliError> {
    let draft = ResolvedDraft::load(path).await?;
    let assets = prepare_assets(&draft).await?;

    let legacy_policy = match legacy_state {
        Some(LegacyStateAction::Adopt) => LegacyStatePolicy::Adopt,
        Some(LegacyStateAction::Discard) => LegacyStatePolicy::Discard,
        None => LegacyStatePolicy::Reject,
    };
    let (_state_lock, mut state) =
        DraftState::acquire(&draft.root, client.account_id(), legacy_policy).await?;
    let mut counts = AssetCounts::default();
    let article = match assets {
        PreparedAssets::News {
            content,
            cover,
            local_images,
        } => {
            let cover_media_id = permanent_media_id(client, &mut state, cover, &mut counts).await?;
            let mut replacements = HashMap::new();
            for LocalArticleImage {
                source,
                path,
                key,
                hash,
            } in local_images
            {
                let url = if let Some(url) = state.article_image(&key, &hash).map(ToOwned::to_owned)
                {
                    counts.reused += 1;
                    url
                } else {
                    let uploaded = client.materials().upload_article_image(&path).await?;
                    let url = uploaded.url;
                    state.record_article_image(key, hash, url.clone());
                    state.save().await?;
                    counts.uploaded += 1;
                    url
                };
                replacements.insert(source, url);
            }
            let content = rewrite_local_images(&content, &replacements)?;
            draft.build_news_article(cover_media_id, content)?
        }
        PreparedAssets::Picture { images } => {
            let mut media_ids = Vec::with_capacity(images.len());
            for image in images {
                media_ids.push(permanent_media_id(client, &mut state, image, &mut counts).await?);
            }
            draft.build_picture_article(media_ids)?
        }
    };
    let request = AddDraftRequest::new(vec![article])?;
    let request_bytes = serde_json::to_vec(&request)?;
    let fingerprint = bytes_sha256(&request_bytes);

    if let Some(media_id) = state.completed_media_id(&fingerprint) {
        return Ok(result_json(
            media_id,
            draft.title(),
            counts.uploaded,
            counts.reused,
            true,
        ));
    }
    if state.has_pending() && !retry_ambiguous {
        return Err(CliError::PendingDraft);
    }

    state.begin_push(fingerprint.clone());
    state.save().await?;
    match client.draft().add(&request).await {
        Ok(response) => {
            state.complete_push(fingerprint, response.media_id.clone());
            let mut result = result_json(
                &response.media_id,
                draft.title(),
                counts.uploaded,
                counts.reused,
                false,
            );
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

struct PermanentAsset {
    path: PathBuf,
    key: String,
    hash: String,
}

struct LocalArticleImage {
    source: String,
    path: PathBuf,
    key: String,
    hash: String,
}

enum PreparedAssets {
    News {
        content: String,
        cover: PermanentAsset,
        local_images: Vec<LocalArticleImage>,
    },
    Picture {
        images: Vec<PermanentAsset>,
    },
}

#[derive(Default)]
struct AssetCounts {
    uploaded: u64,
    reused: u64,
}

async fn prepare_assets(draft: &ResolvedDraft) -> Result<PreparedAssets, CliError> {
    // Resolve, validate, and hash every dependency before uploading anything.
    // This avoids leaving permanent orphan assets when a later path is bad.
    match &draft.article {
        ResolvedArticle::News {
            content,
            cover_path,
            ..
        } => {
            let cover = prepare_permanent_asset(draft, cover_path).await?;
            let mut local_images = Vec::new();
            for source in local_image_sources(content)? {
                let path = draft.resolve_local_image(&source).await?;
                validate_article_image(&path).await?;
                let key = draft.relative_key(&path)?;
                let hash = file_sha256(&path).await?;
                local_images.push(LocalArticleImage {
                    source,
                    path,
                    key,
                    hash,
                });
            }
            Ok(PreparedAssets::News {
                content: content.clone(),
                cover,
                local_images,
            })
        }
        ResolvedArticle::Picture { image_paths, .. } => {
            let mut images = Vec::with_capacity(image_paths.len());
            for path in image_paths {
                images.push(prepare_permanent_asset(draft, path).await?);
            }
            Ok(PreparedAssets::Picture { images })
        }
    }
}

async fn prepare_permanent_asset(
    draft: &ResolvedDraft,
    path: &Path,
) -> Result<PermanentAsset, CliError> {
    validate_permanent_image(path).await?;
    Ok(PermanentAsset {
        path: path.to_path_buf(),
        key: draft.relative_key(path)?,
        hash: file_sha256(path).await?,
    })
}

async fn permanent_media_id(
    client: &WechatOAClient,
    state: &mut DraftState,
    asset: PermanentAsset,
    counts: &mut AssetCounts,
) -> Result<String, CliError> {
    if let Some((media_id, _)) = state
        .permanent_image(&asset.key, &asset.hash)
        .map(|(media_id, url)| (media_id.to_owned(), url.to_owned()))
    {
        counts.reused += 1;
        return Ok(media_id);
    }

    let uploaded = client.materials().upload_image(&asset.path).await?;
    let media_id = uploaded.media_id.to_string();
    state.record_permanent_image(asset.key, asset.hash, media_id.clone(), uploaded.url);
    state.save().await?;
    counts.uploaded += 1;
    Ok(media_id)
}

pub(super) fn is_unambiguous_failure(error: &WechatError) -> bool {
    match error {
        WechatError::Api(_)
        | WechatError::Encode(_)
        | WechatError::InvalidParameter { .. }
        | WechatError::InvalidConfig(_)
        | WechatError::Config(_)
        | WechatError::ConfigRead { .. }
        | WechatError::ConfigDecode { .. }
        | WechatError::InvalidApiEndpoint(_)
        | WechatError::InvalidTokenResponse(_)
        | WechatError::HttpClientBuild(_)
        | WechatError::InsecureConfigPermissions { .. }
        | WechatError::Io(_) => true,
        WechatError::HttpStatus { status, .. } => *status < 500,
        WechatError::Http { .. }
        | WechatError::Decode { .. }
        | WechatError::ResponseTooLarge { .. } => false,
    }
}

fn result_json(
    media_id: &str,
    title: &str,
    uploaded_assets: u64,
    reused_assets: u64,
    reused_draft: bool,
) -> Value {
    json!({
        "media_id": media_id,
        "title": title,
        "reused_draft": reused_draft,
        "state_saved": true,
        "assets": {
            "uploaded": uploaded_assets,
            "reused": reused_assets,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_or_undecodable_creation_responses_remain_ambiguous() {
        let too_large = WechatError::ResponseTooLarge {
            endpoint: "draft/add".to_owned(),
            limit: 10,
        };
        assert!(!is_unambiguous_failure(&too_large));

        let decode = WechatError::Decode {
            endpoint: "draft/add".to_owned(),
            source: serde_json::from_slice::<Value>(b"not-json").unwrap_err(),
        };
        assert!(!is_unambiguous_failure(&decode));
    }
}
