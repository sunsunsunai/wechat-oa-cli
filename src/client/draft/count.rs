use serde::Deserialize;

use crate::{client::draft::DraftApi, error::WechatError};

const COUNT_DRAFTS_ENDPOINT: &str = "https://api.weixin.qq.com/cgi-bin/draft/count";

#[derive(Debug, Deserialize)]
struct CountDraftsResponse {
    total_count: u64,
}

impl DraftApi<'_> {
    pub async fn count(&self) -> Result<u64, WechatError> {
        let response: CountDraftsResponse = self.client.get(COUNT_DRAFTS_ENDPOINT).await?;
        Ok(response.total_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_the_documented_response() {
        let response: CountDraftsResponse = serde_json::from_str(r#"{"total_count":15}"#).unwrap();

        assert_eq!(response.total_count, 15);
    }
}
