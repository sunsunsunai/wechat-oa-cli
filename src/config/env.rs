use crate::error::ConfigError;
use std::env;

pub(crate) fn required_env(name: &'static str) -> Result<String, ConfigError> {
    let value = env::var(name).map_err(|error| match error {
        env::VarError::NotPresent => ConfigError::MissingEnvironmentVariable { name },
        env::VarError::NotUnicode(_) => ConfigError::InvalidEnvironmentVariable { name },
    })?;

    if value.trim().is_empty() {
        return Err(ConfigError::EmptyEnvironmentVariable { name });
    }

    Ok(value.trim().to_owned())
}
