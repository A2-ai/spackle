use super::slot::Slot;
use colored::Colorize;
use fronma::engines::Toml;
use fronma::parser::parse_with_engine;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, fs, io, path::PathBuf};

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub slots: Vec<Slot>,
    #[serde(default)]
    pub hooks: Vec<Hook>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Hook {
    pub key: String,
    pub command: Vec<String>,
    pub r#if: Option<String>,
    pub optional: Option<HookConfigOptional>,
    pub name: Option<String>,
    pub description: Option<String>,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HookConfigOptional {
    pub default: bool,
}

pub const CONFIG_FILE: &str = "spackle.toml";

#[derive(Debug)]
pub enum Error {
    ReadError(io::Error),
    ParseError(toml::de::Error),
    FronmaError(fronma::error::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ReadError(e) => write!(f, "Error reading file\n{}", e),
            Error::ParseError(e) => write!(f, "Error parsing contents\n{}", e),
            Error::FronmaError(e) => write!(f, "Error parsing single file\n{:?}", e),
        }
    }
}

// Loads the config for the given directory
pub fn load_dir(dir: &PathBuf) -> Result<Config, Error> {
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

pub fn load_file(file: &PathBuf) -> Result<Config, Error> {
    let file_contents = match fs::read_to_string(file) {
        Ok(o) => o,
        Err(e) => return Err(Error::ReadError(e)),
    };

    parse_with_engine::<Config, Toml>(&file_contents)
        .map(|parsed| parsed.headers)
        .map_err(Error::FronmaError)
}

#[cfg(test)]
mod tests {
    use tempdir::TempDir;

    use super::*;

    #[test]
    fn load_empty() {
        let dir = TempDir::new("spackle").unwrap().into_path();

        fs::write(&dir.join("spackle.toml"), "").unwrap();

        let result = load_dir(&dir);

        assert!(result.is_ok());
    }
}
