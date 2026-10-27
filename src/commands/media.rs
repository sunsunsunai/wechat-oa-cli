use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::{
    cli::CliError,
    client::{
        WechatOAClient,
        material::{validate_article_image, validate_permanent_image},
    },
    commands::draft::{
        state::{DraftState, file_sha256},
        workflow::is_unambiguous_failure,
    },
    error::WechatError,
};

pub async fn upload_image(
    client: &WechatOAClient,
    path: &Path,
    retry_ambiguous: bool,
    discard_pending: bool,
) -> Result<Value, CliError> {
    let path = canonicalize(path).await?;
    validate_permanent_image(&path).await?;
    protected_upload(
        client,
        &path,
        retry_ambiguous,
        discard_pending,
        "api-material-upload-image",
        "permanent_image",
        || async { client.materials().upload_image(&path).await },
    )
    .await
}

pub async fn upload_article_image(
    client: &WechatOAClient,
    path: &Path,
    retry_ambiguous: bool,
    discard_pending: bool,
) -> Result<Value, CliError> {
    let path = canonicalize(path).await?;
    validate_article_image(&path).await?;
    protected_upload(
        client,
        &path,
        retry_ambiguous,
        discard_pending,
        "api-material-upload-article-image",
        "article_image",
        || async { client.materials().upload_article_image(&path).await },
    )
    .await
}

async fn canonicalize(path: &Path) -> Result<std::path::PathBuf, CliError> {
    tokio::fs::canonicalize(path)
        .await
        .map_err(|source| CliError::ReadFile {
            path: path.to_path_buf(),
            source,
        })
}

async fn protected_upload<T, F, Fut>(
    client: &WechatOAClient,
    path: &Path,
    retry_ambiguous: bool,
    discard_pending: bool,
    operation: &'static str,
    kind: &'static str,
    upload: F,
) -> Result<Value, CliError>
where
    T: Serialize,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, WechatError>>,
{
    let hash = file_sha256(path).await?;
    let (_state_lock, mut state) =
        DraftState::acquire_operation(operation, client.account_id()).await?;
    if discard_pending && (state.has_pending() || state.has_pending_asset_upload()) {
        state.clear_all_pending();
        state.save().await?;
    }
    if state.has_pending_asset_upload() && !retry_ambiguous {
        return Err(CliError::PendingAssetUpload);
    }
    let key = path.to_string_lossy().into_owned();
    if state.has_pending_asset_upload() && !state.pending_asset_upload_matches(kind, &key, &hash) {
        return Err(CliError::PendingAssetUploadMismatch);
    }

    state.begin_asset_upload(kind, key, hash);
    state.save().await?;
    match upload().await {
        Ok(response) => {
            state.clear_pending_asset_upload();
            let mut result = serde_json::to_value(response)?;
            if let Err(error) = state.save().await {
                result["state_saved"] = Value::Bool(false);
                result["warning"] = Value::String(format!(
                    "asset was uploaded remotely, but local state could not be saved: {error}"
                ));
            }
            Ok(result)
        }
        Err(source) if is_unambiguous_failure(&source) => {
            state.clear_pending_asset_upload();
            state.save().await?;
            Err(source.into())
        }
        Err(source) => Err(CliError::AmbiguousAssetUpload { source }),
    }
}
