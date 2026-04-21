use fronma::{engines::Toml, parser::parse_with_engine};
use serde::Deserialize;
use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};
use thiserror::Error;

use crate::fs::FileSystem;
use crate::{hook::Hook, slot::Slot};

#[derive(serde::Serialize, Deserialize, Debug, Default)]
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
    #[error("Error reading file {file:?}:\n{error}")]
    ReadError { file: PathBuf, error: io::Error },
    #[error("Error parsing contents\n{0}")]
    ParseError(toml::de::Error),
    #[error("Error parsing single file\n{0:?}")]
    FronmaError(fronma::error::Error),
    #[error("Duplicate keys found\n{0}")]
    DuplicateKey(String),
}

/// Parse a spackle config from an already-loaded TOML string.
/// Used by WASM bindings where the caller handles file I/O.
pub fn parse(toml_str: &str) -> Result<Config, Error> {
    toml::from_str(toml_str).map_err(Error::ParseError)
}

pub fn load<F: FileSystem>(fs: &F, path: impl AsRef<Path>) -> Result<Config, Error> {
    let path = path.as_ref();
    // Treat as a directory if it lists — otherwise fall back to file.
    // Avoids needing a dedicated `is_dir` method on the trait.
    if let Ok(stat) = fs.stat(path) {
        if stat.file_type == crate::fs::FileType::Directory {
            return load_dir(fs, path);
        }
    }
    load_file(fs, path)
}

// Loads the config for the given directory
pub fn load_dir<F: FileSystem>(fs: &F, dir: impl AsRef<Path>) -> Result<Config, Error> {
    let config_path = dir.as_ref().join(CONFIG_FILE);

    let bytes = fs.read_file(&config_path).map_err(|e| Error::ReadError {
        file: config_path.clone(),
        error: e,
    })?;
    let config_str = String::from_utf8(bytes).map_err(|e| Error::ReadError {
        file: config_path.clone(),
        error: io::Error::new(io::ErrorKind::InvalidData, e.to_string()),
    })?;

    let config = toml::from_str(&config_str).map_err(Error::ParseError)?;

    Ok(config)
}

pub fn load_file<F: FileSystem>(fs: &F, file: impl AsRef<Path>) -> Result<Config, Error> {
    let file = file.as_ref();
    let bytes = fs.read_file(file).map_err(|e| Error::ReadError {
        file: file.to_path_buf(),
        error: e,
    })?;
    let file_contents = String::from_utf8(bytes).map_err(|e| Error::ReadError {
        file: file.to_path_buf(),
        error: io::Error::new(io::ErrorKind::InvalidData, e.to_string()),
    })?;

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
    #[cfg(not(target_arch = "wasm32"))]
    use tempdir::TempDir;

    use super::*;
    use crate::fs::StdFs;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn load_empty() {
        let dir = TempDir::new("spackle").unwrap().into_path();

        std::fs::write(dir.join("spackle.toml"), "").unwrap();

        let result = load_dir(&StdFs::new(), &dir);

        assert!(result.is_ok());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dup_key() {
        let dir = Path::new("tests/data/conf_dup_key");

        let config = load_dir(&StdFs::new(), dir).expect("Expected ok");

        config.validate().expect_err("Expected error");
    }

    // --- Table-driven tests for parse() ---

    #[test]
    fn parse_table() {
        struct Case {
            name: &'static str,
            toml: &'static str,
            expect_ok: bool,
            expect_slots: usize,
            expect_hooks: usize,
        }

        let cases = vec![
            Case {
                name: "empty config",
                toml: "",
                expect_ok: true,
                expect_slots: 0,
                expect_hooks: 0,
            },
            Case {
                name: "one slot",
                toml: r#"
[[slots]]
key = "name"
type = "String"
"#,
                expect_ok: true,
                expect_slots: 1,
                expect_hooks: 0,
            },
            Case {
                name: "slot + hook",
                toml: r#"
[[slots]]
key = "x"
type = "Number"
default = "42"

[[hooks]]
key = "init"
command = ["echo", "hi"]
default = true
"#,
                expect_ok: true,
                expect_slots: 1,
                expect_hooks: 1,
            },
            Case {
                name: "with name and ignore",
                toml: r#"
name = "my-project"
ignore = [".git", "target"]

[[slots]]
key = "a"
"#,
                expect_ok: true,
                expect_slots: 1,
                expect_hooks: 0,
            },
            Case {
                name: "invalid toml",
                toml: "[[[ broken",
                expect_ok: false,
                expect_slots: 0,
                expect_hooks: 0,
            },
        ];

        for c in cases {
            let result = parse(c.toml);
            assert_eq!(result.is_ok(), c.expect_ok, "case {}", c.name);
            if let Ok(cfg) = result {
                assert_eq!(cfg.slots.len(), c.expect_slots, "case {}: slots", c.name);
                assert_eq!(cfg.hooks.len(), c.expect_hooks, "case {}: hooks", c.name);
            }
        }
    }
}
