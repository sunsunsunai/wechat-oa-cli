use serde_json::{Value, json};

use crate::{cli::CliError, client::WechatOAClient};

pub async fn check(client: &WechatOAClient) -> Result<Value, CliError> {
    client.check_auth().await?;
    Ok(json!({ "authenticated": true }))
}
