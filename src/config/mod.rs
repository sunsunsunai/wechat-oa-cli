pub mod env;
pub mod storage;

use std::{env as std_env, path::Path};

use crate::{client::token::TokenManager, error::WechatError};

const APP_ID_ENV_KEY: &str = "WECHAT_OA_APP_ID";
const APP_SECRET_ENV_KEY: &str = "WECHAT_OA_APP_SECRET";

pub fn load_token_manager(config_path: Option<&Path>) -> Result<TokenManager, WechatError> {
    let builder = TokenManager::builder();

    if let Some(path) = config_path {
        return builder.load_json(path)?.build();
    }

    if std_env::var_os(APP_ID_ENV_KEY).is_some() || std_env::var_os(APP_SECRET_ENV_KEY).is_some() {
        return builder.load_env()?.build();
    }

    builder.load_json(storage::default_config_path()?)?.build()
}
