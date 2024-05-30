use serde::Deserialize;
use std::{fs, io, path::PathBuf};

use super::slot::Slot;

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub ignore: Vec<String>,
    pub slots: Vec<Slot>,
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
