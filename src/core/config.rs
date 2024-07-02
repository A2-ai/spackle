use super::slot::Slot;
use colored::Colorize;
use serde::Deserialize;
use std::{fmt::Display, fs, io, path::PathBuf};

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub ignore: Vec<String>,
    pub slots: Vec<Slot>,
    pub hooks: Vec<Hook>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Hook {
    pub key: String,
    pub command: Vec<String>,
    pub r#if: Option<String>,
    /// Should hook be user-toggleable?
    pub optional: Option<HookConfigOptional>,
}

impl Display for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}\n{}",
            self.key.bold(),
            if let Some(optional) = &self.optional {
                format!(
                    "optional, default {}",
                    if optional.default {
                        "on".green()
                    } else {
                        "off".red()
                    }
                )
            } else {
                "".to_string()
            }
            .dimmed(),
            self.command
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
                .join(" ")
                .dimmed()
        )
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct HookConfigOptional {
    /// Whether the hook is enabled by default.
    pub default: bool,
}

pub const CONFIG_FILE: &str = "spackle.toml";

#[derive(Debug)]
pub enum Error {
    ReadError(io::Error),
    ParseError(toml::de::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ReadError(e) => write!(f, "Error reading file\n{}", e),
            Error::ParseError(e) => write!(f, "Error parsing contents\n{}", e),
        }
    }
}

// Loads the config for the given directory
pub fn load(dir: &PathBuf) -> Result<Config, Error> {
    let config_path = dir.join(CONFIG_FILE);

    let config_str = match fs::read_to_string(config_path) {
        Ok(o) => o,
        Err(e) => return Err(Error::ReadError(e)),
    };

    let config = match toml::from_str(&config_str) {
        Ok(o) => o,
        Err(e) => return Err(Error::ParseError(e)),
    };

    Ok(config)
}
