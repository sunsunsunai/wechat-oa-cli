use std::{fs, io::Read, path::Path};

use serde::Deserialize;

use super::TokenManager;
use crate::{config::env::required_env, error::WechatError};

const APP_ID_ENV_KEY: &str = "WECHAT_OA_APP_ID";
const APP_SECRET_ENV_KEY: &str = "WECHAT_OA_APP_SECRET";
const MAX_CONFIG_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
struct TokenConfig {
    app_id: String,
    app_secret: String,
}

pub struct TokenManagerBuilder {
    app_id: Option<String>,
    app_secret: Option<String>,
}

impl TokenManagerBuilder {
    pub fn new() -> Self {
        Self {
            app_id: None,
            app_secret: None,
        }
    }

    pub fn app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = Some(app_id.into());
        self
    }

    pub fn app_secret(mut self, app_secret: impl Into<String>) -> Self {
        self.app_secret = Some(app_secret.into());
        self
    }

    pub fn load_env(self) -> Result<Self, WechatError> {
        let app_id = required_env(APP_ID_ENV_KEY)?;
        let app_secret = required_env(APP_SECRET_ENV_KEY)?;

        Ok(self.app_id(app_id).app_secret(app_secret))
    }

    pub fn load_json(self, path: impl AsRef<Path>) -> Result<Self, WechatError> {
        let path = path.as_ref();
        let file = fs::File::open(path).map_err(|source| WechatError::ConfigRead {
            path: path.to_path_buf(),
            source,
        })?;
        let metadata = file.metadata().map_err(|source| WechatError::ConfigRead {
            path: path.to_path_buf(),
            source,
        })?;
        ensure_private_config(path, &metadata)?;
        let mut content = Vec::with_capacity(
            usize::try_from(metadata.len().min(MAX_CONFIG_BYTES as u64))
                .unwrap_or(MAX_CONFIG_BYTES),
        );
        file.take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut content)
            .map_err(|source| WechatError::ConfigRead {
                path: path.to_path_buf(),
                source,
            })?;
        if content.len() > MAX_CONFIG_BYTES {
            return Err(WechatError::InvalidConfig(
                "config file must not exceed 64 KiB",
            ));
        }
        let config: TokenConfig =
            serde_json::from_slice(&content).map_err(|source| WechatError::ConfigDecode {
                path: path.to_path_buf(),
                source,
            })?;

        Ok(self.app_id(config.app_id).app_secret(config.app_secret))
    }

    pub fn build(self) -> Result<TokenManager, WechatError> {
        let app_id = self
            .app_id
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or(WechatError::InvalidConfig("app_id is required"))?;

        let app_secret = self
            .app_secret
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or(WechatError::InvalidConfig("app_secret is required"))?;

        Ok(TokenManager::new(app_id, app_secret))
    }
}

#[cfg(unix)]
fn ensure_private_config(path: &Path, metadata: &fs::Metadata) -> Result<(), WechatError> {
    use std::os::unix::fs::MetadataExt;

    let mode = metadata.mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(WechatError::InsecureConfigPermissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_config(_path: &Path, _metadata: &fs::Metadata) -> Result<(), WechatError> {
    Ok(())
}

impl Default for TokenManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_whitespace_only_credentials() {
        let error = TokenManagerBuilder::new()
            .app_id("   ")
            .app_secret("secret")
            .build()
            .err()
            .expect("whitespace-only app IDs must be rejected");
        assert!(matches!(error, WechatError::InvalidConfig(_)));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_credential_files_readable_by_other_users() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "wechat-oa-insecure-config-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, br#"{"app_id":"id","app_secret":"secret"}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let error = TokenManagerBuilder::new()
            .load_json(&path)
            .err()
            .expect("insecure configuration must be rejected");
        let _ = fs::remove_file(path);

        assert!(matches!(
            error,
            WechatError::InsecureConfigPermissions { mode: 0o644, .. }
        ));
    }
}
