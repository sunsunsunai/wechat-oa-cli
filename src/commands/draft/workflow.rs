use std::{collections::HashMap, path::Path};

use serde_json::{Value, json};

use super::{
    document::ResolvedDraft,
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
    validate_permanent_image(&draft.cover_path).await?;

    // Resolve and validate every local dependency before uploading anything.
    // This avoids leaving permanent orphan assets when a later path is bad.
    let mut local_images = Vec::new();
    for source in local_image_sources(&draft.content)? {
        let image_path = draft.resolve_local_image(&source).await?;
        validate_article_image(&image_path).await?;
        let key = draft.relative_key(&image_path)?;
        let hash = file_sha256(&image_path).await?;
        local_images.push((source, image_path, key, hash));
    }

    let legacy_policy = match legacy_state {
        Some(LegacyStateAction::Adopt) => LegacyStatePolicy::Adopt,
        Some(LegacyStateAction::Discard) => LegacyStatePolicy::Discard,
        None => LegacyStatePolicy::Reject,
    };
    let (_state_lock, mut state) =
        DraftState::acquire(&draft.root, client.account_id(), legacy_policy).await?;
    let mut uploaded_assets = 0_u64;
    let mut reused_assets = 0_u64;

    let cover_key = draft.relative_key(&draft.cover_path)?;
    let cover_hash = file_sha256(&draft.cover_path).await?;
    let cover_media_id = if let Some((media_id, _)) = state
        .permanent_image(&cover_key, &cover_hash)
        .map(|(media_id, url)| (media_id.to_owned(), url.to_owned()))
    {
        reused_assets += 1;
        media_id
    } else {
        let uploaded = client.materials().upload_image(&draft.cover_path).await?;
        let media_id = uploaded.media_id.to_string();
        state.record_permanent_image(cover_key, cover_hash, media_id.clone(), uploaded.url);
        state.save().await?;
        uploaded_assets += 1;
        media_id
    };

    let mut replacements = HashMap::new();
    for (source, image_path, key, hash) in local_images {
        let url = if let Some(url) = state.article_image(&key, &hash).map(ToOwned::to_owned) {
            reused_assets += 1;
            url
        } else {
            let uploaded = client.materials().upload_article_image(&image_path).await?;
            let url = uploaded.url;
            state.record_article_image(key, hash, url.clone());
            state.save().await?;
            uploaded_assets += 1;
            url
        };
        replacements.insert(source, url);
    }

    let content = rewrite_local_images(&draft.content, &replacements)?;
    let article = draft.build_article(cover_media_id, content)?;
    let request = AddDraftRequest::new(vec![article.into()])?;
    let request_bytes = serde_json::to_vec(&request)?;
    let fingerprint = bytes_sha256(&request_bytes);

    if let Some(media_id) = state.completed_media_id(&fingerprint) {
        return Ok(result_json(
            media_id,
            draft.title(),
            uploaded_assets,
            reused_assets,
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
                uploaded_assets,
                reused_assets,
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
