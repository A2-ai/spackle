use std::{collections::HashMap, fmt::Display};

use super::config::{Slot, SlotType};

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
                write!(f, "type mismatch for key {}: {}", key, r#type)
            }
            Error::UndefinedSlot(key) => write!(f, "slot was not defined: {}", key),
        }
    }
}

pub fn validate(data: &HashMap<String, String>, slots: Vec<Slot>) -> Result<(), Error> {
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
