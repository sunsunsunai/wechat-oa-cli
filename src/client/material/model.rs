use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::error::WechatError;

/// A media ID created by the permanent-material API.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PermanentMediaId(String);

impl PermanentMediaId {
    pub fn new(value: impl Into<String>) -> Result<Self, WechatError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(WechatError::InvalidParameter {
                name: "media_id",
                message: "cannot be empty",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for PermanentMediaId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for PermanentMediaId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PermanentMediaId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermanentImage {
    pub media_id: PermanentMediaId,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArticleImage {
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_media_id_serializes_as_a_string() {
        let media_id = PermanentMediaId::new("media-id").unwrap();

        assert_eq!(serde_json::to_value(media_id).unwrap(), "media-id");
    }

    #[test]
    fn rejects_an_empty_permanent_media_id() {
        assert!(PermanentMediaId::new("  ").is_err());
        assert!(serde_json::from_str::<PermanentMediaId>(r#""  ""#).is_err());
    }
}
