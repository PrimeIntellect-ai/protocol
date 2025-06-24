use eyre::{Context, Result};
use std::env;
use std::io::{self, Write};

pub enum KeySource {
    Environment(String),
    File(String),
    Interactive,
}

impl KeySource {
    pub fn from_arg(key_arg: Option<String>) -> Self {
        match key_arg {
            Some(key) if key.starts_with("env:") => {
                Self::Environment(key.strip_prefix("env:").unwrap().to_string())
            }
            Some(key) if key.starts_with("file:") => {
                Self::File(key.strip_prefix("file:").unwrap().to_string())
            }
            Some(_) => {
                eprintln!(
                    "Warning: Direct private key arguments are deprecated for security reasons."
                );
                eprintln!("Use 'env:VAR_NAME' or 'file:/path/to/key' instead.");
                Self::Interactive
            }
            None => Self::Interactive,
        }
    }

    pub fn resolve(&self) -> Result<String> {
        match self {
            Self::Environment(var_name) => env::var(var_name)
                .with_context(|| format!("Environment variable {} not found", var_name)),
            Self::File(path) => std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read private key from file: {}", path))
                .map(|s| s.trim().to_string()),
            Self::Interactive => {
                print!("Enter private key (hidden): ");
                io::stdout().flush().unwrap();
                rpassword::read_password().context("Failed to read private key")
            }
        }
    }
}

pub fn get_private_key(key_arg: Option<String>) -> Result<String> {
    let key_source = KeySource::from_arg(key_arg);
    key_source.resolve()
}
