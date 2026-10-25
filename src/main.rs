use std::process::ExitCode;

use clap::Parser;
use wechat_oa_cli::{
    cli::{Cli, CliError, output},
    commands,
};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if !error.use_stderr() {
                let _ = error.print();
                return ExitCode::SUCCESS;
            }
            let error = CliError::Arguments(error.to_string());
            let _ = output::write_error(&error, false);
            return ExitCode::from(output::exit_code(&error));
        }
    };
    let pretty = cli.pretty;

    match commands::run(cli).await {
        Ok(value) => match output::write_success(&value, pretty) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        },
        Err(error) => {
            let exit_code = output::exit_code(&error);
            let _ = output::write_error(&error, pretty);
            ExitCode::from(exit_code)
        }
    }
}
