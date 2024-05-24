use colored::{Color, Colorize};
use serde::Deserialize;
use std::fmt::Display;
use std::{error::Error, fs, path::PathBuf};

#[derive(Deserialize, Debug)]
pub struct Config {
    pub slots: Vec<Slot>,
}

#[derive(Deserialize, Debug)]
pub struct Slot {
    pub key: String,
    pub r#type: SlotType,
    pub name: String,
    pub description: String,
}

#[derive(Deserialize, Debug, strum_macros::Display)]
pub enum SlotType {
    Number,
    String,
    Boolean,
}

impl Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}\n{}",
            self.key.yellow(),
            // mid gray color
            ("[".to_owned() + &self.r#type.to_string() + "]")
                .to_string()
                .to_lowercase()
                .truecolor(128, 128, 128),
            self.description.truecolor(180, 180, 180),
        )
    }
}

const CONFIG_FILE: &str = "spackle.toml";

// Loads the config for the given directory
pub fn load(dir: &PathBuf) -> Result<Config, Box<dyn Error>> {
    let config_path = dir.join(CONFIG_FILE);

    let config_str = match fs::read_to_string(config_path) {
        Ok(o) => o,
        Err(e) => return Err(format!("Failed to read config file: {}", e).into()),
    };

    let config = match toml::from_str(&config_str) {
        Ok(o) => o,
        Err(e) => return Err(format!("Failed to parse config file: {}", e).into()),
    };

    Ok(config)
}

// Ensure the data is consistent with slot definition
// pub fn validate_slot_data(
//     slots: Vec<Slot>,
//     data: Vec<(String, String)>,
// ) -> Result<(), Box<dyn Error>> {
//     slots.into_iter().map(|slot| {
//         // Check data type
//     })
// }
