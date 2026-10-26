use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use super::html::local_image_sources;
use crate::{
    cli::CliError,
    client::{
        draft::{DraftArticleInput, NewsArticleInput, PictureArticleInput},
        material::{PermanentMediaId, validate_article_image, validate_permanent_image},
    },
};

const MANIFEST_FILE_NAME: &str = "draft.json";
const SUPPORTED_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "article_type", deny_unknown_fields)]
enum DraftManifest {
    #[serde(rename = "news")]
    News {
        schema_version: u32,
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
    },
    #[serde(rename = "newspic")]
    Picture {
        schema_version: u32,
        title: String,
        content_file: PathBuf,
        image_files: Vec<PathBuf>,
        #[serde(default)]
        need_open_comment: bool,
        #[serde(default)]
        only_fans_can_comment: bool,
    },
}

pub(crate) enum ResolvedArticle {
    News {
        title: String,
        content: String,
        cover_path: PathBuf,
        author: Option<String>,
        digest: Option<String>,
        content_source_url: Option<String>,
        need_open_comment: bool,
        only_fans_can_comment: bool,
    },
    Picture {
        title: String,
        content: String,
        image_paths: Vec<PathBuf>,
        need_open_comment: bool,
        only_fans_can_comment: bool,
    },
}

pub(crate) struct ResolvedDraft {
    pub(crate) root: PathBuf,
    pub(crate) manifest_path: PathBuf,
    pub(crate) article: ResolvedArticle,
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
        let article = match manifest {
            DraftManifest::News {
                schema_version,
                title,
                content_file,
                cover_file,
                author,
                digest,
                content_source_url,
                need_open_comment,
                only_fans_can_comment,
            } => {
                validate_schema_version(schema_version)?;
                validate_comment_options(need_open_comment, only_fans_can_comment)?;
                let content = read_content(&root, &content_file).await?;
                let cover_path = resolve_file_inside(&root, &cover_file).await?;
                ResolvedArticle::News {
                    title,
                    content,
                    cover_path,
                    author,
                    digest,
                    content_source_url,
                    need_open_comment,
                    only_fans_can_comment,
                }
            }
            DraftManifest::Picture {
                schema_version,
                title,
                content_file,
                image_files,
                need_open_comment,
                only_fans_can_comment,
            } => {
                validate_schema_version(schema_version)?;
                validate_comment_options(need_open_comment, only_fans_can_comment)?;
                let content = read_content(&root, &content_file).await?;
                let mut image_paths = Vec::with_capacity(image_files.len());
                for image_file in image_files {
                    image_paths.push(resolve_file_inside(&root, &image_file).await?);
                }
                ResolvedArticle::Picture {
                    title,
                    content,
                    image_paths,
                    need_open_comment,
                    only_fans_can_comment,
                }
            }
        };

        let draft = Self {
            root,
            manifest_path,
            article,
        };
        draft.validate_article_fields()?;
        Ok(draft)
    }

    pub(crate) async fn resolve_local_image(&self, source: &str) -> Result<PathBuf, CliError> {
        let path = resolve_inside(&self.root, Path::new(source)).await?;
        ensure_file(&path).await?;
        Ok(path)
    }

    pub(crate) fn build_news_article(
        &self,
        thumb_media_id: impl Into<String>,
        content: String,
    ) -> Result<DraftArticleInput, CliError> {
        let ResolvedArticle::News {
            title,
            author,
            digest,
            content_source_url,
            need_open_comment,
            only_fans_can_comment,
            ..
        } = &self.article
        else {
            return Err(CliError::InvalidDraft(
                "cannot build a news article from a newspic manifest".to_owned(),
            ));
        };
        let mut article = NewsArticleInput::new(
            title.clone(),
            content,
            PermanentMediaId::new(thumb_media_id)?,
        )?
        .open_comments(*need_open_comment)
        .fans_only_comments(*only_fans_can_comment);
        if let Some(author) = author {
            article = article.author(author.clone())?;
        }
        if let Some(digest) = digest {
            article = article.digest(digest.clone())?;
        }
        if let Some(url) = content_source_url {
            article = article.content_source_url(url.clone())?;
        }
        Ok(article.into())
    }

    pub(crate) fn build_picture_article(
        &self,
        image_media_ids: Vec<String>,
    ) -> Result<DraftArticleInput, CliError> {
        let ResolvedArticle::Picture {
            title,
            content,
            need_open_comment,
            only_fans_can_comment,
            ..
        } = &self.article
        else {
            return Err(CliError::InvalidDraft(
                "cannot build a newspic article from a news manifest".to_owned(),
            ));
        };
        let image_media_ids = image_media_ids
            .into_iter()
            .map(PermanentMediaId::new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(
            PictureArticleInput::new(title.clone(), content.clone(), image_media_ids)?
                .open_comments(*need_open_comment)
                .fans_only_comments(*only_fans_can_comment)
                .into(),
        )
    }

    pub(crate) fn title(&self) -> &str {
        match &self.article {
            ResolvedArticle::News { title, .. } | ResolvedArticle::Picture { title, .. } => title,
        }
    }

    pub(crate) fn article_type(&self) -> &'static str {
        match &self.article {
            ResolvedArticle::News { .. } => "news",
            ResolvedArticle::Picture { .. } => "newspic",
        }
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

    fn validate_article_fields(&self) -> Result<(), CliError> {
        match &self.article {
            ResolvedArticle::News { content, .. } => {
                self.build_news_article("validation-media-id", content.clone())?;
            }
            ResolvedArticle::Picture { image_paths, .. } => {
                self.build_picture_article(
                    (0..image_paths.len())
                        .map(|index| format!("validation-media-id-{index}"))
                        .collect(),
                )?;
            }
        }
        Ok(())
    }
}

pub async fn validate(path: &Path) -> Result<Value, CliError> {
    let draft = ResolvedDraft::load(path).await?;
    let mut result = json!({
        "valid": true,
        "manifest": draft.relative_key(&draft.manifest_path)?,
        "article_type": draft.article_type(),
        "title": draft.title(),
    });
    match &draft.article {
        ResolvedArticle::News {
            content,
            cover_path,
            ..
        } => {
            validate_permanent_image(cover_path).await?;
            let local_images = local_image_sources(content)?;
            let mut resolved_images = Vec::with_capacity(local_images.len());
            for source in local_images {
                let image_path = draft.resolve_local_image(&source).await?;
                validate_article_image(&image_path).await?;
                resolved_images.push(draft.relative_key(&image_path)?);
            }
            result["cover"] = Value::String(draft.relative_key(cover_path)?);
            result["local_images"] = serde_json::to_value(resolved_images)?;
        }
        ResolvedArticle::Picture { image_paths, .. } => {
            let mut images = Vec::with_capacity(image_paths.len());
            for image_path in image_paths {
                validate_permanent_image(image_path).await?;
                images.push(draft.relative_key(image_path)?);
            }
            result["images"] = serde_json::to_value(images)?;
        }
    }
    Ok(result)
}

pub fn schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "WeChat Official Account local draft",
        "oneOf": [news_schema(), picture_schema()],
        "examples": [news_example(), picture_example()]
    })
}

fn news_schema() -> Value {
    json!({
        "title": "Article draft",
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
        }
    })
}

fn picture_schema() -> Value {
    json!({
        "title": "Picture draft",
        "type": "object",
        "additionalProperties": false,
        "required": ["schema_version", "article_type", "title", "content_file", "image_files"],
        "properties": {
            "schema_version": { "const": 1 },
            "article_type": { "const": "newspic" },
            "title": { "type": "string", "minLength": 1, "maxLength": 32 },
            "content_file": { "type": "string" },
            "image_files": {
                "type": "array",
                "minItems": 1,
                "maxItems": 20,
                "items": { "type": "string" }
            },
            "need_open_comment": { "type": "boolean", "default": false },
            "only_fans_can_comment": { "type": "boolean", "default": false }
        }
    })
}

fn news_example() -> Value {
    json!({
        "schema_version": 1,
        "article_type": "news",
        "title": "Article title",
        "content_file": "content.html",
        "cover_file": "assets/cover.jpg",
        "need_open_comment": false,
        "only_fans_can_comment": false
    })
}

fn picture_example() -> Value {
    json!({
        "schema_version": 1,
        "article_type": "newspic",
        "title": "Picture post title",
        "content_file": "content.txt",
        "image_files": ["assets/01.jpg", "assets/02.jpg"],
        "need_open_comment": false,
        "only_fans_can_comment": false
    })
}

fn validate_comment_options(
    need_open_comment: bool,
    only_fans_can_comment: bool,
) -> Result<(), CliError> {
    if only_fans_can_comment && !need_open_comment {
        return Err(CliError::InvalidDraft(
            "only_fans_can_comment requires need_open_comment to be true".to_owned(),
        ));
    }
    Ok(())
}

fn validate_schema_version(schema_version: u32) -> Result<(), CliError> {
    if schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(CliError::InvalidDraft(format!(
            "unsupported schema_version {schema_version}; expected {SUPPORTED_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

async fn read_content(root: &Path, relative: &Path) -> Result<String, CliError> {
    let content_path = resolve_file_inside(root, relative).await?;
    let content_bytes = read_limited(&content_path, MAX_CONTENT_BYTES).await?;
    String::from_utf8(content_bytes).map_err(|_| {
        CliError::InvalidDraft(format!(
            "content file {} is not valid UTF-8",
            content_path.display()
        ))
    })
}

async fn resolve_file_inside(root: &Path, relative: &Path) -> Result<PathBuf, CliError> {
    let path = resolve_inside(root, relative).await?;
    ensure_file(&path).await?;
    Ok(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_draft_preserves_image_media_id_order() {
        let draft = ResolvedDraft {
            root: PathBuf::from("."),
            manifest_path: PathBuf::from("draft.json"),
            article: ResolvedArticle::Picture {
                title: "Picture post".to_owned(),
                content: "Caption".to_owned(),
                image_paths: vec![PathBuf::from("01.jpg"), PathBuf::from("02.jpg")],
                need_open_comment: false,
                only_fans_can_comment: false,
            },
        };

        let article = draft
            .build_picture_article(vec!["media-1".to_owned(), "media-2".to_owned()])
            .unwrap();
        let value = serde_json::to_value(article).unwrap();

        assert_eq!(value["article_type"], "newspic");
        assert_eq!(
            value["image_info"]["image_list"],
            json!([
                { "image_media_id": "media-1" },
                { "image_media_id": "media-2" }
            ])
        );
    }
}
