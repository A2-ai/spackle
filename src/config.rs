use fronma::{engines::Toml, parser::parse_with_engine};
use serde::Deserialize;
use std::{collections::HashSet, fs, io, path::Path};
use thiserror::Error;

use crate::{hook::Hook, slot::Slot};

#[derive(Deserialize, Debug, Default)]
pub struct Config {
    pub name: Option<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub slots: Vec<Slot>,
    #[serde(default)]
    pub hooks: Vec<Hook>,
}

pub const CONFIG_FILE: &str = "spackle.toml";

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error reading file\n{0}")]
    ReadError(io::Error),
    #[error("Error parsing contents\n{0}")]
    ParseError(toml::de::Error),
    #[error("Error parsing single file\n{0:?}")]
    FronmaError(fronma::error::Error),
    #[error("Duplicate keys found\n{0}")]
    DuplicateKey(String),
}

pub fn load(path: impl AsRef<Path>) -> Result<Config, Error> {
    if path.as_ref().is_dir() {
        return load_dir(path);
    }

    load_file(path)
}

// Loads the config for the given directory
pub fn load_dir(dir: impl AsRef<Path>) -> Result<Config, Error> {
    let config_path = dir.as_ref().join(CONFIG_FILE);

    let config_str = fs::read_to_string(config_path).map_err(Error::ReadError)?;

    let config = toml::from_str(&config_str).map_err(Error::ParseError)?;

    Ok(config)
}

pub fn load_file(file: impl AsRef<Path>) -> Result<Config, Error> {
    let file_contents = fs::read_to_string(file).map_err(Error::ReadError)?;

    parse_with_engine::<Config, Toml>(&file_contents)
        .map(|parsed| parsed.headers)
        .map_err(Error::FronmaError)
}

impl Config {
    pub fn validate(&self) -> Result<(), Error> {
        let hook_keys: HashSet<&String> = self.hooks.iter().map(|hook| &hook.key).collect();
        let slot_keys: HashSet<&String> = self.slots.iter().map(|slot| &slot.key).collect();

        let shared_keys: HashSet<_> = hook_keys.intersection(&slot_keys).collect();

        if !shared_keys.is_empty() {
            return Err(Error::DuplicateKey(
                shared_keys
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
                    .join(", "),
            ));
        }

        // Check for duplicate keys within hooks
        if hook_keys.len() != self.hooks.len() {
            return Err(Error::DuplicateKey(
                "Duplicate keys found in hooks".to_string(),
            ));
        }

        // Check for duplicate keys within slots
        if slot_keys.len() != self.slots.len() {
            return Err(Error::DuplicateKey(
                "Duplicate keys found in slots".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempdir::TempDir;

    use super::*;

    #[test]
    fn load_empty() {
        let dir = TempDir::new("spackle").unwrap().into_path();

        fs::write(dir.join("spackle.toml"), "").unwrap();

        let result = load_dir(&dir);

        assert!(result.is_ok());
    }

    #[test]
    fn dup_key() {
        let dir = Path::new("tests/data/conf_dup_key");

        let config = load_dir(dir).expect("Expected ok");

        config.validate().expect_err("Expected error");
    }
}
