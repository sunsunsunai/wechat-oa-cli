use std::path::PathBuf;

use directories::BaseDirs;

use crate::error::WechatError;

pub fn default_config_path() -> Result<PathBuf, WechatError> {
    let base_dirs = BaseDirs::new().ok_or(WechatError::InvalidConfig(
        "cannot determine the user configuration directory",
    ))?;

    Ok(base_dirs.config_dir().join("wechat-oa-cli/config.json"))
}
