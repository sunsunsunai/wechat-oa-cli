use std::path::Path;

use tokio::io::AsyncReadExt;

use super::{ArticleImage, MaterialApi, PermanentImage};
use crate::error::WechatError;

const UPLOAD_PERMANENT_IMAGE_ENDPOINT: &str =
    "https://api.weixin.qq.com/cgi-bin/material/add_material?type=image";
const UPLOAD_ARTICLE_IMAGE_ENDPOINT: &str = "https://api.weixin.qq.com/cgi-bin/media/uploadimg";
const MAX_PERMANENT_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_ARTICLE_IMAGE_BYTES: usize = 1024 * 1024;

pub(crate) async fn validate_permanent_image(path: &Path) -> Result<(), WechatError> {
    ImageFile::load(path, ImagePurpose::Permanent, MAX_PERMANENT_IMAGE_BYTES)
        .await
        .map(drop)
}

pub(crate) async fn validate_article_image(path: &Path) -> Result<(), WechatError> {
    ImageFile::load(path, ImagePurpose::Article, MAX_ARTICLE_IMAGE_BYTES)
        .await
        .map(drop)
}

impl MaterialApi<'_> {
    /// Uploads an image to the permanent material library.
    pub async fn upload_image(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<PermanentImage, WechatError> {
        let file = ImageFile::load(
            path.as_ref(),
            ImagePurpose::Permanent,
            MAX_PERMANENT_IMAGE_BYTES,
        )
        .await?;

        self.client
            .post_multipart_once(
                UPLOAD_PERMANENT_IMAGE_ENDPOINT,
                "media",
                file.name,
                file.mime_type,
                file.data,
            )
            .await
    }

    /// Uploads an image for embedding in article HTML and returns its URL.
    pub async fn upload_article_image(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ArticleImage, WechatError> {
        let file = ImageFile::load(
            path.as_ref(),
            ImagePurpose::Article,
            MAX_ARTICLE_IMAGE_BYTES,
        )
        .await?;

        self.client
            .post_multipart_once(
                UPLOAD_ARTICLE_IMAGE_ENDPOINT,
                "media",
                file.name,
                file.mime_type,
                file.data,
            )
            .await
    }
}

#[derive(Clone, Copy)]
enum ImagePurpose {
    Permanent,
    Article,
}

struct ImageFile {
    name: String,
    mime_type: &'static str,
    data: Vec<u8>,
}

impl ImageFile {
    async fn load(
        path: &Path,
        purpose: ImagePurpose,
        max_bytes: usize,
    ) -> Result<Self, WechatError> {
        let name = path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or(WechatError::InvalidParameter {
                name: "path",
                message: "must point to an image file",
            })?
            .to_string_lossy()
            .into_owned();
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or(WechatError::InvalidParameter {
                name: "path",
                message: "image file must have a supported extension",
            })?;
        let mime_type = image_mime_type(&extension, purpose)?;
        let metadata = tokio::fs::metadata(path).await?;
        if metadata.len() >= max_bytes as u64 {
            return Err(image_too_large(purpose));
        }

        // Limit the read as well as checking metadata so a concurrently replaced
        // file cannot make the process allocate an unbounded buffer.
        let file = tokio::fs::File::open(path).await?;
        let mut data = Vec::with_capacity(metadata.len() as usize);
        file.take(max_bytes as u64).read_to_end(&mut data).await?;

        if data.is_empty() {
            return Err(WechatError::InvalidParameter {
                name: "path",
                message: "image file cannot be empty",
            });
        }
        if data.len() >= max_bytes {
            return Err(image_too_large(purpose));
        }
        validate_image_signature(&data, &extension)?;

        Ok(Self {
            name,
            mime_type,
            data,
        })
    }
}

fn image_too_large(purpose: ImagePurpose) -> WechatError {
    WechatError::InvalidParameter {
        name: "path",
        message: match purpose {
            ImagePurpose::Permanent => "permanent image must be smaller than 10 MiB",
            ImagePurpose::Article => "article image must be smaller than 1 MiB",
        },
    }
}

fn validate_image_signature(data: &[u8], extension: &str) -> Result<(), WechatError> {
    let valid = match extension {
        "png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
        "gif" => data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"),
        "bmp" => data.starts_with(b"BM"),
        _ => false,
    };
    if !valid {
        return Err(WechatError::InvalidParameter {
            name: "path",
            message: "file contents do not match the image extension",
        });
    }
    Ok(())
}

fn image_mime_type(extension: &str, purpose: ImagePurpose) -> Result<&'static str, WechatError> {
    let mime_type = match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "bmp" if matches!(purpose, ImagePurpose::Permanent) => "image/bmp",
        "gif" if matches!(purpose, ImagePurpose::Permanent) => "image/gif",
        _ => {
            return Err(WechatError::InvalidParameter {
                name: "path",
                message: match purpose {
                    ImagePurpose::Permanent => {
                        "permanent image must be BMP, PNG, JPEG, JPG, or GIF"
                    }
                    ImagePurpose::Article => "article image must be PNG, JPEG, or JPG",
                },
            });
        }
    };
    Ok(mime_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_documented_permanent_image_formats() {
        for extension in ["bmp", "png", "jpeg", "jpg", "gif"] {
            assert!(image_mime_type(extension, ImagePurpose::Permanent).is_ok());
        }
    }

    #[test]
    fn article_images_reject_formats_not_supported_by_uploadimg() {
        assert!(image_mime_type("gif", ImagePurpose::Article).is_err());
        assert!(image_mime_type("bmp", ImagePurpose::Article).is_err());
    }

    #[test]
    fn rejects_a_file_whose_signature_does_not_match_its_extension() {
        assert!(validate_image_signature(b"not a png", "png").is_err());
        assert!(validate_image_signature(b"\x89PNG\r\n\x1a\n", "png").is_ok());
    }
}
