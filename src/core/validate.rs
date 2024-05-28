use std::collections::HashMap;

use super::config::{Slot, SlotType};

pub enum Error {
    InvalidData(String),
    DataIsWrongType(String, String),
    SlotIsMissingData(String),
}

pub fn validate(data: &HashMap<String, String>, slots: Vec<Slot>) -> Result<(), Error> {
    for entry in data.iter() {
        // Check if the data is assigned to a slot
        let slot = match slots.iter().find(|slot| slot.key == *entry.0) {
            Some(slot) => slot,
            None => {
                return Err(Error::InvalidData(entry.0.clone()));
            }
        };

        // Verify the data type by trying to parse it as the slot type
        if !match slot.r#type {
            SlotType::String => entry.1.parse::<String>().is_ok(),
            SlotType::Number => entry.1.parse::<f64>().is_ok(),
            SlotType::Boolean => entry.1.parse::<bool>().is_ok(),
        } {
            return Err(Error::DataIsWrongType(
                entry.0.clone(),
                slot.r#type.to_string(),
            ));
        }
    }

    // Ensure all slots are assigned data
    for slot in slots.iter() {
        if !data.iter().any(|data| *data.0 == slot.key) {
            return Err(Error::SlotIsMissingData(slot.key.clone()));
        }
    }

    Ok(())
}
