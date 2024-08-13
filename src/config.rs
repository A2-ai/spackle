use serde::Deserialize;
use std::{collections::HashSet, fmt::Display, fs, io, path::PathBuf};

use crate::{hook::Hook, slot::Slot};

#[derive(Deserialize, Debug)]
#[derive(Default)]
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

#[derive(Debug)]
pub enum Error {
    ErrorReading(io::Error),
    ErrorParsing(toml::de::Error),
    DuplicateKey(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ErrorReading(e) => write!(f, "Error reading file\n{}", e),
            Error::ErrorParsing(e) => write!(f, "Error parsing contents\n{}", e),
            Error::DuplicateKey(key) => write!(f, "Duplicate key: {}", key),
        }
    }
}

// Loads the config for the given directory
pub fn load(dir: &PathBuf) -> Result<Config, Error> {
    let config_path = dir.join(CONFIG_FILE);

    let config_str = match fs::read_to_string(config_path) {
        Ok(o) => o,
        Err(e) => return Err(Error::ErrorReading(e)),
    };

    let config = match toml::from_str(&config_str) {
        Ok(o) => o,
        Err(e) => return Err(Error::ErrorParsing(e)),
    };

    Ok(config)
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

        let result = load(&dir);

        assert!(result.is_ok());
    }

    #[test]
    fn dup_key() {
        let dir = PathBuf::from("tests/data/conf_dup_key");

        let config = load(&dir).expect("Expected ok");

        config.validate().expect_err("Expected error");
    }
}
