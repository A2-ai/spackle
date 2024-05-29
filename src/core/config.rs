use serde::Deserialize;
use std::{error::Error, fs, path::PathBuf};

use super::slot::Slot;

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub ignore: Vec<String>,
    pub slots: Vec<Slot>,
}

pub const CONFIG_FILE: &str = "spackle.toml";

// Loads the config for the given directory
pub fn load(dir: &PathBuf) -> Result<Config, Box<dyn Error>> {
    let config_path = dir.join(CONFIG_FILE);

    let config_str = match fs::read_to_string(config_path) {
        Ok(o) => o,
        Err(e) => return Err(format!("Failed to read config file\n{}", e).into()),
    };

    let config = match toml::from_str(&config_str) {
        Ok(o) => o,
        Err(e) => return Err(format!("Failed to parse config file\n{}", e).into()),
    };

    Ok(config)
}
