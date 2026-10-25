pub mod auth;
pub mod draft;
pub mod media;

use serde_json::Value;

use crate::{
    cli::{
        ApiCommand, ApiDraftCommand, ApiMaterialCommand, AuthCommand, Cli, CliError, Command,
        DraftCommand,
    },
    client::WechatOAClient,
    config,
};

pub async fn run(cli: Cli) -> Result<Value, CliError> {
    let Cli {
        config: config_path,
        command,
        ..
    } = cli;

    if let Command::Draft { command } = &command {
        match command {
            DraftCommand::Validate { path } => return draft::validate(path).await,
            DraftCommand::Schema => return Ok(draft::schema()),
            _ => {}
        }
    }

    let token_manager = config::load_token_manager(config_path.as_deref())?;
    let client = WechatOAClient::builder(token_manager).build()?;

    match command {
        Command::Auth {
            command: AuthCommand::Check,
        } => auth::check(&client).await,
        Command::Draft { command } => match command {
            DraftCommand::Push {
                path,
                retry_ambiguous,
                legacy_state,
            } => draft::push(&client, &path, retry_ambiguous, legacy_state).await,
            DraftCommand::Count => draft::count(&client).await,
            DraftCommand::List(args) => {
                draft::list(&client, args.offset, args.count, args.include_content).await
            }
            DraftCommand::Get { media_id } => draft::get(&client, &media_id).await,
            DraftCommand::Validate { .. } | DraftCommand::Schema => unreachable!(),
        },
        Command::Api { command } => match command {
            ApiCommand::Material { command } => match command {
                ApiMaterialCommand::UploadImage { path } => {
                    media::upload_image(&client, &path).await
                }
                ApiMaterialCommand::UploadArticleImage { path } => {
                    media::upload_article_image(&client, &path).await
                }
            },
            ApiCommand::Draft {
                command:
                    ApiDraftCommand::Add {
                        input,
                        retry_ambiguous,
                    },
            } => draft::add_from_input(&client, &input, retry_ambiguous).await,
        },
    }
}
