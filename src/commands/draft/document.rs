use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use super::html::local_image_sources;
use crate::{
    cli::CliError,
    client::{
        draft::NewsArticleInput,
        material::{PermanentMediaId, validate_article_image, validate_permanent_image},
    },
};

const MANIFEST_FILE_NAME: &str = "draft.json";
const SUPPORTED_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftManifest {
    schema_version: u32,
    article_type: ArticleType,
    title: String,
    content_file: PathBuf,
    cover_file: PathBuf,
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ArticleType {
    News,
}

pub(crate) struct ResolvedDraft {
    pub(crate) root: PathBuf,
    pub(crate) manifest_path: PathBuf,
    pub(crate) content: String,
    pub(crate) cover_path: PathBuf,
    manifest: DraftManifest,
}

impl ResolvedDraft {
    pub(crate) async fn load(path: &Path) -> Result<Self, CliError> {
        let input = canonicalize(path).await?;
        let metadata = tokio::fs::metadata(&input)
            .await
            .map_err(|source| CliError::ReadFile {
                path: input.clone(),
                source,
            })?;
        let (root, manifest_path) = if metadata.is_dir() {
            let manifest_path = resolve_inside(&input, Path::new(MANIFEST_FILE_NAME)).await?;
            (input.clone(), manifest_path)
        } else {
            let root = input.parent().ok_or_else(|| {
                CliError::InvalidDraft("manifest has no parent directory".to_owned())
            })?;
            (root.to_path_buf(), input)
        };

        let manifest_bytes = read_limited(&manifest_path, MAX_MANIFEST_BYTES).await?;
        let manifest: DraftManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|source| CliError::DecodeInput {
                source_name: manifest_path.display().to_string(),
                source,
            })?;
        if manifest.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(CliError::InvalidDraft(format!(
                "unsupported schema_version {}; expected {}",
                manifest.schema_version, SUPPORTED_SCHEMA_VERSION
            )));
        }
        if manifest.only_fans_can_comment && !manifest.need_open_comment {
            return Err(CliError::InvalidDraft(
                "only_fans_can_comment requires need_open_comment to be true".to_owned(),
            ));
        }
        match manifest.article_type {
            ArticleType::News => {}
        }

        let content_path = resolve_inside(&root, &manifest.content_file).await?;
        let cover_path = resolve_inside(&root, &manifest.cover_file).await?;
        ensure_file(&content_path).await?;
        ensure_file(&cover_path).await?;
        let content_bytes = read_limited(&content_path, MAX_CONTENT_BYTES).await?;
        let content = String::from_utf8(content_bytes).map_err(|_| {
            CliError::InvalidDraft(format!(
                "content file {} is not valid UTF-8",
                content_path.display()
            ))
        })?;

        let draft = Self {
            root,
            manifest_path,
            content,
            cover_path,
            manifest,
        };
        draft.build_article("validation-media-id", draft.content.clone())?;
        Ok(draft)
    }

    pub(crate) async fn resolve_local_image(&self, source: &str) -> Result<PathBuf, CliError> {
        let path = resolve_inside(&self.root, Path::new(source)).await?;
        ensure_file(&path).await?;
        Ok(path)
    }

    pub(crate) fn build_article(
        &self,
        thumb_media_id: impl Into<String>,
        content: String,
    ) -> Result<NewsArticleInput, CliError> {
        let mut article = NewsArticleInput::new(
            self.manifest.title.clone(),
            content,
            PermanentMediaId::new(thumb_media_id)?,
        )?
        .open_comments(self.manifest.need_open_comment)
        .fans_only_comments(self.manifest.only_fans_can_comment);
        if let Some(author) = &self.manifest.author {
            article = article.author(author.clone())?;
        }
        if let Some(digest) = &self.manifest.digest {
            article = article.digest(digest.clone())?;
        }
        if let Some(url) = &self.manifest.content_source_url {
            article = article.content_source_url(url.clone())?;
        }
        Ok(article)
    }

    pub(crate) fn title(&self) -> &str {
        &self.manifest.title
    }

    pub(crate) fn relative_key(&self, path: &Path) -> Result<String, CliError> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            CliError::InvalidDraft(format!(
                "asset {} is outside the draft directory",
                path.display()
            ))
        })?;
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }
}

pub async fn validate(path: &Path) -> Result<Value, CliError> {
    let draft = ResolvedDraft::load(path).await?;
    validate_permanent_image(&draft.cover_path).await?;
    let local_images = local_image_sources(&draft.content)?;
    let mut resolved_images = Vec::with_capacity(local_images.len());
    for source in local_images {
        let image_path = draft.resolve_local_image(&source).await?;
        validate_article_image(&image_path).await?;
        resolved_images.push(draft.relative_key(&image_path)?);
    }

    Ok(json!({
        "valid": true,
        "manifest": draft.relative_key(&draft.manifest_path)?,
        "title": draft.title(),
        "cover": draft.relative_key(&draft.cover_path)?,
        "local_images": resolved_images,
    }))
}

pub fn schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "WeChat Official Account local draft",
        "type": "object",
        "additionalProperties": false,
        "required": ["schema_version", "article_type", "title", "content_file", "cover_file"],
        "properties": {
            "schema_version": { "const": 1 },
            "article_type": { "const": "news" },
            "title": { "type": "string", "minLength": 1, "maxLength": 32 },
            "content_file": { "type": "string" },
            "cover_file": { "type": "string" },
            "author": { "type": "string", "maxLength": 16 },
            "digest": { "type": "string", "maxLength": 120 },
            "content_source_url": { "type": "string" },
            "need_open_comment": { "type": "boolean", "default": false },
            "only_fans_can_comment": { "type": "boolean", "default": false }
        },
        "example": {
            "schema_version": 1,
            "article_type": "news",
            "title": "Article title",
            "content_file": "content.html",
            "cover_file": "assets/cover.jpg",
            "need_open_comment": false,
            "only_fans_can_comment": false
        }
    })
}

async fn resolve_inside(root: &Path, relative: &Path) -> Result<PathBuf, CliError> {
    if relative.is_absolute() {
        return Err(CliError::InvalidDraft(format!(
            "path {} must be relative to the draft directory",
            relative.display()
        )));
    }
    let resolved = canonicalize(&root.join(relative)).await?;
    if !resolved.starts_with(root) {
        return Err(CliError::InvalidDraft(format!(
            "path {} resolves outside the draft directory",
            relative.display()
        )));
    }
    Ok(resolved)
}

async fn canonicalize(path: &Path) -> Result<PathBuf, CliError> {
    tokio::fs::canonicalize(path)
        .await
        .map_err(|source| CliError::ReadFile {
            path: path.to_path_buf(),
            source,
        })
}

async fn ensure_file(path: &Path) -> Result<(), CliError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|source| CliError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(CliError::InvalidDraft(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    Ok(())
}

async fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>, CliError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|source| CliError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|source| CliError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > limit {
        return Err(CliError::InputTooLarge { limit });
    }
    Ok(bytes)
}
