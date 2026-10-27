use std::io::{self, Write};

use serde::Serialize;
use serde_json::{Value, json};

use super::CliError;
use crate::error::WechatError;

pub fn write_success(value: &Value, pretty: bool) -> io::Result<()> {
    write_json(io::stdout().lock(), value, pretty)
}

pub fn write_error(error: &CliError, pretty: bool) -> io::Result<()> {
    let value = error_json(error);
    write_json(io::stderr().lock(), &value, pretty)
}

pub fn exit_code(error: &CliError) -> u8 {
    match error {
        CliError::Arguments(_) => 2,
        CliError::Wechat(
            WechatError::Config(_)
            | WechatError::ConfigDecode { .. }
            | WechatError::ConfigRead { .. }
            | WechatError::InsecureConfigPermissions { .. }
            | WechatError::InvalidConfig(_),
        ) => 2,
        CliError::DecodeInput { .. }
        | CliError::InvalidDate { .. }
        | CliError::InvalidDraft(_)
        | CliError::Html(_)
        | CliError::InputTooLarge { .. }
        | CliError::LegacyStatePolicyRequired
        | CliError::ConflictingLegacyStates
        | CliError::Wechat(WechatError::InvalidParameter { .. }) => 3,
        CliError::Wechat(WechatError::Api(error)) if error.is_permission_denied() => 5,
        CliError::Wechat(WechatError::Api(_)) => 4,
        CliError::PendingDraft
        | CliError::PendingDraftMismatch
        | CliError::PendingAssetUpload
        | CliError::PendingAssetUploadMismatch
        | CliError::AmbiguousDraftPush { .. }
        | CliError::AmbiguousAssetUpload { .. } => 6,
        _ => 1,
    }
}

fn error_json(error: &CliError) -> Value {
    match error {
        CliError::Wechat(WechatError::Api(api_error)) => json!({
            "error": {
                "type": "wechat_api",
                "message": api_error.message,
                "code": api_error.code,
                "request_id": api_error.request_id,
            }
        }),
        _ => json!({
            "error": {
                "type": error_type(error),
                "message": error.to_string(),
            }
        }),
    }
}

fn error_type(error: &CliError) -> &'static str {
    match error {
        CliError::Arguments(_) => "invalid_arguments",
        CliError::Wechat(
            WechatError::Config(_)
            | WechatError::ConfigDecode { .. }
            | WechatError::ConfigRead { .. }
            | WechatError::InsecureConfigPermissions { .. }
            | WechatError::InvalidConfig(_),
        ) => "configuration",
        CliError::Wechat(
            WechatError::Http { .. }
            | WechatError::HttpStatus { .. }
            | WechatError::ResponseTooLarge { .. },
        ) => "http",
        CliError::Wechat(WechatError::Api(_)) => "wechat_api",
        CliError::Wechat(WechatError::InvalidParameter { .. })
        | CliError::DecodeInput { .. }
        | CliError::InvalidDate { .. }
        | CliError::InvalidDraft(_)
        | CliError::Html(_)
        | CliError::InputTooLarge { .. } => "invalid_input",
        CliError::LegacyStatePolicyRequired | CliError::ConflictingLegacyStates => "legacy_state",
        CliError::ReadFile { .. } | CliError::WriteFile { .. } | CliError::LocalState(_) => "io",
        CliError::PendingDraft
        | CliError::PendingDraftMismatch
        | CliError::PendingAssetUpload
        | CliError::PendingAssetUploadMismatch
        | CliError::AmbiguousDraftPush { .. }
        | CliError::AmbiguousAssetUpload { .. } => "ambiguous_operation",
        _ => "internal",
    }
}

fn write_json(mut writer: impl Write, value: &impl Serialize, pretty: bool) -> io::Result<()> {
    if pretty {
        serde_json::to_writer_pretty(&mut writer, value).map_err(io::Error::other)?;
    } else {
        serde_json::to_writer(&mut writer, value).map_err(io::Error::other)?;
    }
    writer.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::WechatApiError;

    #[test]
    fn preserves_wechat_error_fields() {
        let error = CliError::Wechat(WechatError::Api(WechatApiError {
            code: 48_001,
            message: "api unauthorized".to_owned(),
            request_id: Some("request-id".to_owned()),
        }));

        let value = error_json(&error);
        assert_eq!(value["error"]["type"], "wechat_api");
        assert_eq!(value["error"]["code"], 48_001);
        assert_eq!(value["error"]["request_id"], "request-id");
        assert_eq!(exit_code(&error), 5);
    }

    #[test]
    fn classifies_oversized_input_as_invalid_input() {
        let error = CliError::InputTooLarge { limit: 42 };

        assert_eq!(error_type(&error), "invalid_input");
        assert_eq!(exit_code(&error), 3);
    }
}
