use colored::Colorize;
use serde::Deserialize;
use std::{collections::HashMap, fmt::Display};

#[derive(Deserialize, Debug)]
pub struct Slot {
    pub key: String,
    pub r#type: SlotType,
    pub name: Option<String>,
    pub description: Option<String>,
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
            ("[".to_owned() + &self.r#type.to_string() + "]")
                .to_string()
                .to_lowercase()
                .truecolor(128, 128, 128),
            self.description
                .clone()
                .unwrap_or("no description".to_string())
                .truecolor(180, 180, 180),
        )
    }
}

#[derive(Debug)]
pub enum Error {
    UnknownSlot(String),
    TypeMismatch(String, String),
    UndefinedSlot(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnknownSlot(key) => write!(f, "unknown slot: {}", key),
            Error::TypeMismatch(key, r#type) => {
                write!(f, "type mismatch for key {}: expected a {}", key, r#type)
            }
            Error::UndefinedSlot(key) => write!(f, "slot was not defined: {}", key),
        }
    }
}

pub fn validate_data(data: &HashMap<String, String>, slots: Vec<Slot>) -> Result<(), Error> {
    for entry in data.iter() {
        // Check if the data is assigned to a slot
        let slot = match slots.iter().find(|slot| slot.key == *entry.0) {
            Some(slot) => slot,
            None => {
                return Err(Error::UnknownSlot(entry.0.clone()));
            }
        };

        // Verify the data type by trying to parse it as the slot type
        if !match slot.r#type {
            SlotType::String => entry.1.parse::<String>().is_ok(),
            SlotType::Number => entry.1.parse::<f64>().is_ok(),
            SlotType::Boolean => entry.1.parse::<bool>().is_ok(),
        } {
            return Err(Error::TypeMismatch(
                entry.0.clone(),
                slot.r#type.to_string(),
            ));
        }
    }

    // Ensure all slots are assigned data
    for slot in slots.iter() {
        if !data.iter().any(|data| *data.0 == slot.key) {
            return Err(Error::UndefinedSlot(slot.key.clone()));
        }
    }

    Ok(())
}
