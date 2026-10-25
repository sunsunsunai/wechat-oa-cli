use std::{io, path::PathBuf};

use thiserror::Error;

use crate::error::WechatError;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("invalid command line: {0}")]
    Arguments(String),

    #[error(transparent)]
    Wechat(#[from] WechatError),

    #[error("failed to read {}: {source}", path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to write {}: {source}", path.display())]
    WriteFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to decode JSON from {source_name}: {source}")]
    DecodeInput {
        source_name: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to encode JSON: {0}")]
    EncodeJson(#[from] serde_json::Error),

    #[error("invalid date `{value}`; expected YYYY-MM-DD")]
    InvalidDate { value: String },

    #[error("invalid draft bundle: {0}")]
    InvalidDraft(String),

    #[error("failed to process draft HTML: {0}")]
    Html(String),

    #[error("local operation state error: {0}")]
    LocalState(String),

    #[error("input exceeds the {limit} byte limit")]
    InputTooLarge { limit: usize },

    #[error(
        "legacy draft state is not tied to an account; rerun with `--legacy-state adopt` only if it belongs to the current account, or `--legacy-state discard` to rebuild it"
    )]
    LegacyStatePolicyRequired,

    #[error(
        "legacy draft state conflicts with existing state or another legacy copy; rerun with `--legacy-state discard` to rebuild the cache"
    )]
    ConflictingLegacyStates,

    #[error(
        "the previous draft request may have reached WeChat; inspect remote drafts, then pass --retry-ambiguous only if creating another draft is acceptable"
    )]
    PendingDraft,

    #[error(
        "draft creation may have reached WeChat but its response was not received: {source}; inspect remote drafts before retrying"
    )]
    AmbiguousDraftPush {
        #[source]
        source: WechatError,
    },
}
