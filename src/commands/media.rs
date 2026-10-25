use std::path::Path;

use serde_json::Value;

use crate::{cli::CliError, client::WechatOAClient};

pub async fn upload_image(client: &WechatOAClient, path: &Path) -> Result<Value, CliError> {
    let response = client.materials().upload_image(path).await?;
    Ok(serde_json::to_value(response)?)
}

pub async fn upload_article_image(client: &WechatOAClient, path: &Path) -> Result<Value, CliError> {
    let response = client.materials().upload_article_image(path).await?;
    Ok(serde_json::to_value(response)?)
}
