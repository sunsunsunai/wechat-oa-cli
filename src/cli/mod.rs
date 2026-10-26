mod error;
pub mod output;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

pub use error::CliError;

#[derive(Debug, Parser)]
#[command(
    name = "wechat-oa",
    version,
    about = "A machine-friendly CLI for WeChat Official Accounts"
)]
pub struct Cli {
    /// Path to a JSON file containing app_id and app_secret.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Pretty-print JSON output. Compact JSON is the default for automation.
    #[arg(long, global = true)]
    pub pretty: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Verify credentials without printing an access token.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Validate, push, and inspect drafts.
    Draft {
        #[command(subcommand)]
        command: DraftCommand,
    },
    /// Low-level one-to-one API operations for advanced use.
    Api {
        #[command(subcommand)]
        command: ApiCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Verify that the configured app ID and secret can obtain a token.
    Check,
}

#[derive(Debug, Subcommand)]
pub enum DraftCommand {
    /// Validate a local draft bundle without accessing WeChat.
    Validate { path: PathBuf },
    /// Upload a local draft bundle and create a remote draft.
    Push {
        path: PathBuf,
        /// Ignore reusable draft and asset records, upload all assets, and create a new draft.
        #[arg(long)]
        force: bool,
        /// Retry when the previous request may have reached WeChat but no response was received.
        #[arg(long)]
        retry_ambiguous: bool,
        /// Decide how to handle draft state created before caches were scoped by account.
        #[arg(long, value_enum)]
        legacy_state: Option<LegacyStateAction>,
    },
    /// Print the local draft manifest schema and an example.
    Schema,
    /// Count remote drafts.
    Count,
    /// List remote drafts.
    List(DraftListArgs),
    /// Get one remote draft by media ID.
    Get { media_id: String },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LegacyStateAction {
    /// Assign the single legacy cache to the currently configured account.
    Adopt,
    /// Delete legacy caches and rebuild them for the current account.
    Discard,
}

#[derive(Debug, Args)]
pub struct DraftListArgs {
    #[arg(long, default_value_t = 0)]
    pub offset: u64,
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=20))]
    pub count: u8,
    /// Include unpublished article bodies in the response.
    #[arg(long)]
    pub include_content: bool,
}

#[derive(Debug, Subcommand)]
pub enum ApiCommand {
    /// Low-level permanent-material and article-image operations.
    Material {
        #[command(subcommand)]
        command: ApiMaterialCommand,
    },
    /// Low-level draft write operations.
    Draft {
        #[command(subcommand)]
        command: ApiDraftCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ApiMaterialCommand {
    /// Upload one image to the permanent material library.
    UploadImage { path: PathBuf },
    /// Upload one image for embedding in article HTML.
    UploadArticleImage { path: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum ApiDraftCommand {
    /// Create a draft from a fully resolved JSON request; use '-' for stdin.
    Add {
        #[arg(long, value_name = "PATH|-", default_value = "-")]
        input: PathBuf,
        /// Retry when the previous request may have reached WeChat but no response was received.
        #[arg(long)]
        retry_ambiguous: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_machine_friendly_draft_commands() {
        let cli = Cli::try_parse_from([
            "wechat-oa",
            "--pretty",
            "draft",
            "list",
            "--offset",
            "20",
            "--count",
            "5",
            "--include-content",
        ])
        .unwrap();

        assert!(cli.pretty);
        assert!(matches!(
            cli.command,
            Command::Draft {
                command: DraftCommand::List(DraftListArgs {
                    offset: 20,
                    count: 5,
                    include_content: true
                })
            }
        ));
    }

    #[test]
    fn rejects_invalid_list_counts_before_network_access() {
        assert!(Cli::try_parse_from(["wechat-oa", "draft", "list", "--count", "21"]).is_err());
    }

    #[test]
    fn draft_list_omits_content_by_default() {
        let cli = Cli::try_parse_from(["wechat-oa", "draft", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Draft {
                command: DraftCommand::List(DraftListArgs {
                    include_content: false,
                    ..
                })
            }
        ));
    }

    #[test]
    fn parses_explicit_legacy_state_action() {
        let cli = Cli::try_parse_from([
            "wechat-oa",
            "draft",
            "push",
            "article",
            "--legacy-state",
            "adopt",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Draft {
                command: DraftCommand::Push {
                    force: false,
                    legacy_state: Some(LegacyStateAction::Adopt),
                    ..
                }
            }
        ));
    }

    #[test]
    fn parses_force_push() {
        let cli =
            Cli::try_parse_from(["wechat-oa", "draft", "push", "article", "--force"]).unwrap();

        assert!(matches!(
            cli.command,
            Command::Draft {
                command: DraftCommand::Push { force: true, .. }
            }
        ));
    }
}
