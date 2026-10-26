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
        draft::{AddDraftRequest, AddDraftResponse},
        material::{
            ArticleImage, PermanentImage, validate_article_image, validate_permanent_image,
        },
    },
    error::WechatError,
};

pub async fn push(
    client: &WechatOAClient,
    path: &Path,
    force: bool,
    retry_ambiguous: bool,
    legacy_state: Option<LegacyStateAction>,
) -> Result<Value, CliError> {
    push_with_backend(client, path, force, retry_ambiguous, legacy_state).await
}

trait DraftBackend {
    fn account_id(&self) -> &str;

    async fn upload_permanent_image(&self, path: &Path) -> Result<PermanentImage, WechatError>;

    async fn upload_article_image(&self, path: &Path) -> Result<ArticleImage, WechatError>;

    async fn add_draft(&self, request: &AddDraftRequest) -> Result<AddDraftResponse, WechatError>;

    async fn draft_exists(&self, media_id: &str) -> Result<bool, WechatError>;
}

impl DraftBackend for WechatOAClient {
    fn account_id(&self) -> &str {
        WechatOAClient::account_id(self)
    }

    async fn upload_permanent_image(&self, path: &Path) -> Result<PermanentImage, WechatError> {
        self.materials().upload_image(path).await
    }

    async fn upload_article_image(&self, path: &Path) -> Result<ArticleImage, WechatError> {
        self.materials().upload_article_image(path).await
    }

    async fn add_draft(&self, request: &AddDraftRequest) -> Result<AddDraftResponse, WechatError> {
        self.draft().add(request).await
    }

    async fn draft_exists(&self, media_id: &str) -> Result<bool, WechatError> {
        match self.draft().get(media_id).await {
            Ok(_) => Ok(true),
            Err(WechatError::Api(error)) if error.code == 40007 => Ok(false),
            Err(error) => Err(error),
        }
    }
}

async fn push_with_backend<B: DraftBackend>(
    backend: &B,
    path: &Path,
    force: bool,
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
        DraftState::acquire(&draft.root, backend.account_id(), legacy_policy).await?;
    if state.has_pending() && !retry_ambiguous {
        return Err(CliError::PendingDraft);
    }

    let mut counts = AssetCounts::default();
    let article = match assets {
        PreparedAssets::News {
            content,
            cover,
            local_images,
        } => {
            let cover_media_id =
                permanent_media_id(backend, &mut state, cover, &mut counts, force).await?;
            let mut replacements = HashMap::new();
            for LocalArticleImage {
                source,
                path,
                key,
                hash,
            } in local_images
            {
                let url = if !force
                    && let Some(url) = state.article_image(&key, &hash).map(ToOwned::to_owned)
                {
                    counts.reused += 1;
                    url
                } else {
                    let uploaded = backend.upload_article_image(&path).await?;
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
                media_ids.push(
                    permanent_media_id(backend, &mut state, image, &mut counts, force).await?,
                );
            }
            draft.build_picture_article(media_ids)?
        }
    };
    let request = AddDraftRequest::new(vec![article])?;
    let request_bytes = serde_json::to_vec(&request)?;
    let fingerprint = bytes_sha256(&request_bytes);

    if !force
        && let Some(media_id) = state
            .completed_media_id(&fingerprint)
            .map(ToOwned::to_owned)
    {
        if backend.draft_exists(&media_id).await? {
            return Ok(result_json(
                &media_id,
                draft.title(),
                counts.uploaded,
                counts.reused,
                true,
            ));
        }
        state.remove_completed(&fingerprint);
        state.save().await?;
    }
    state.begin_push(fingerprint.clone());
    state.save().await?;
    match backend.add_draft(&request).await {
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

async fn permanent_media_id<B: DraftBackend>(
    backend: &B,
    state: &mut DraftState,
    asset: PermanentAsset,
    counts: &mut AssetCounts,
    force: bool,
) -> Result<String, CliError> {
    if !force
        && let Some((media_id, _)) = state
            .permanent_image(&asset.key, &asset.hash)
            .map(|(media_id, url)| (media_id.to_owned(), url.to_owned()))
    {
        counts.reused += 1;
        return Ok(media_id);
    }

    let uploaded = backend.upload_permanent_image(&asset.path).await?;
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
    use std::{
        fs,
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::client::material::PermanentMediaId;

    struct FakeBackend {
        account_id: String,
        permanent_uploads: Mutex<Vec<String>>,
        article_uploads: Mutex<Vec<String>>,
        draft_requests: Mutex<Vec<Value>>,
        draft_exists: Mutex<bool>,
    }

    impl FakeBackend {
        fn new(account_id: String) -> Self {
            Self {
                account_id,
                permanent_uploads: Mutex::new(Vec::new()),
                article_uploads: Mutex::new(Vec::new()),
                draft_requests: Mutex::new(Vec::new()),
                draft_exists: Mutex::new(true),
            }
        }
    }

    impl DraftBackend for FakeBackend {
        fn account_id(&self) -> &str {
            &self.account_id
        }

        async fn upload_permanent_image(&self, path: &Path) -> Result<PermanentImage, WechatError> {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            self.permanent_uploads.lock().unwrap().push(name.clone());
            Ok(PermanentImage {
                media_id: PermanentMediaId::new(format!("media-{name}"))?,
                url: format!("https://example.invalid/{name}"),
            })
        }

        async fn upload_article_image(&self, path: &Path) -> Result<ArticleImage, WechatError> {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            self.article_uploads.lock().unwrap().push(name.clone());
            Ok(ArticleImage {
                url: format!("https://example.invalid/{name}"),
            })
        }

        async fn add_draft(
            &self,
            request: &AddDraftRequest,
        ) -> Result<AddDraftResponse, WechatError> {
            self.draft_requests
                .lock()
                .unwrap()
                .push(serde_json::to_value(request).unwrap());
            Ok(AddDraftResponse {
                media_id: "draft-media-id".to_owned(),
            })
        }

        async fn draft_exists(&self, _media_id: &str) -> Result<bool, WechatError> {
            Ok(*self.draft_exists.lock().unwrap())
        }
    }

    fn temporary_picture_draft(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "wechat-oa-workflow-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(
            root.join("draft.json"),
            r#"{
                "schema_version": 1,
                "article_type": "newspic",
                "title": "Picture post",
                "content_file": "content.txt",
                "image_files": ["assets/01.jpg", "assets/02.png"]
            }"#,
        )
        .unwrap();
        fs::write(root.join("content.txt"), "Picture caption").unwrap();
        fs::write(root.join("assets/01.jpg"), [0xff, 0xd8, 0xff, 0xd9]).unwrap();
        fs::write(root.join("assets/02.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        root
    }

    fn clean_test_state(root: &Path, account_id: &str) {
        let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let directories = DraftState::test_directories(&canonical_root, account_id).unwrap();
        let _ = fs::remove_dir_all(root);
        for directory in directories {
            let _ = fs::remove_dir_all(directory);
        }
    }

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

    #[tokio::test]
    async fn picture_push_uploads_in_order_and_reuses_cached_result() {
        let root = temporary_picture_draft("complete");
        let backend = FakeBackend::new(format!("picture-account-{}", std::process::id()));

        let first = push_with_backend(&backend, &root, false, false, None)
            .await
            .unwrap();
        let second = push_with_backend(&backend, &root, false, false, None)
            .await
            .unwrap();

        assert_eq!(first["media_id"], "draft-media-id");
        assert_eq!(first["assets"]["uploaded"], 2);
        assert_eq!(second["reused_draft"], true);
        assert_eq!(second["assets"]["reused"], 2);
        assert_eq!(
            *backend.permanent_uploads.lock().unwrap(),
            vec!["01.jpg", "02.png"]
        );
        assert!(backend.article_uploads.lock().unwrap().is_empty());

        let requests = backend.draft_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]["articles"][0]["image_info"]["image_list"],
            json!([
                { "image_media_id": "media-01.jpg" },
                { "image_media_id": "media-02.png" }
            ])
        );
        drop(requests);
        clean_test_state(&root, backend.account_id());
    }

    #[tokio::test]
    async fn pending_push_stops_before_uploading_assets() {
        let root = temporary_picture_draft("pending");
        let backend = FakeBackend::new(format!("pending-account-{}", std::process::id()));
        let canonical_root = fs::canonicalize(&root).unwrap();
        let (lock, mut state) = DraftState::acquire(
            &canonical_root,
            backend.account_id(),
            LegacyStatePolicy::Reject,
        )
        .await
        .unwrap();
        state.begin_push("previous-request".to_owned());
        state.save().await.unwrap();
        drop(lock);

        let error = push_with_backend(&backend, &root, false, false, None)
            .await
            .unwrap_err();

        assert!(matches!(error, CliError::PendingDraft));
        assert!(backend.permanent_uploads.lock().unwrap().is_empty());
        assert!(backend.article_uploads.lock().unwrap().is_empty());
        assert!(backend.draft_requests.lock().unwrap().is_empty());
        clean_test_state(&root, backend.account_id());
    }

    #[tokio::test]
    async fn missing_remote_draft_is_recreated() {
        let root = temporary_picture_draft("missing-remote");
        let backend = FakeBackend::new(format!("missing-account-{}", std::process::id()));

        push_with_backend(&backend, &root, false, false, None)
            .await
            .unwrap();
        *backend.draft_exists.lock().unwrap() = false;
        let recreated = push_with_backend(&backend, &root, false, false, None)
            .await
            .unwrap();

        assert_eq!(recreated["reused_draft"], false);
        assert_eq!(recreated["assets"]["reused"], 2);
        assert_eq!(backend.draft_requests.lock().unwrap().len(), 2);
        clean_test_state(&root, backend.account_id());
    }

    #[tokio::test]
    async fn force_reuploads_assets_and_creates_a_new_draft() {
        let root = temporary_picture_draft("force");
        let backend = FakeBackend::new(format!("force-account-{}", std::process::id()));

        push_with_backend(&backend, &root, false, false, None)
            .await
            .unwrap();
        let forced = push_with_backend(&backend, &root, true, false, None)
            .await
            .unwrap();

        assert_eq!(forced["reused_draft"], false);
        assert_eq!(forced["assets"]["uploaded"], 2);
        assert_eq!(forced["assets"]["reused"], 0);
        assert_eq!(backend.permanent_uploads.lock().unwrap().len(), 4);
        assert_eq!(backend.draft_requests.lock().unwrap().len(), 2);
        clean_test_state(&root, backend.account_id());
    }
}
