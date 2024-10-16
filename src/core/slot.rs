use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Display};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Slot {
    pub key: String,
    #[serde(default)]
    pub r#type: SlotType,
    pub name: Option<String>,
    pub description: Option<String>,
    pub default: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, strum_macros::Display, Default, Clone)]
pub enum SlotType {
    Number,
    #[default]
    String,
    Boolean,
}

impl Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}{}",
            self.key.bold(),
            ("[".to_owned() + &self.r#type.to_string() + "]")
                .to_string()
                .to_lowercase()
                .truecolor(128, 128, 128),
            self.description
                .clone()
                .map(|s| format!("\n{}", s))
                .unwrap_or_default()
                .truecolor(180, 180, 180),
        )
    }
}

impl Default for Slot {
    fn default() -> Self {
        Slot {
            key: "".to_string(),
            r#type: SlotType::String,
            name: None,
            description: None,
            default: None,
        }
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

impl Slot {
    pub fn get_name(&self) -> String {
        self.name.clone().unwrap_or(self.key.clone())
    }
}

pub fn validate_data(data: &HashMap<String, String>, slots: &Vec<Slot>) -> Result<(), Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let slots = vec![];

        let data = HashMap::new();

        assert!(validate_data(&data, &slots).is_ok());
    }

    #[test]
    fn valid() {
        let slots = vec![
            Slot {
                key: "key".to_string(),
                ..Default::default()
            },
            Slot {
                key: "key2".to_string(),
                ..Default::default()
            },
        ];

        let data = HashMap::from([("key", "value"), ("key2", "value2")])
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<String, String>>();

        assert!(validate_data(&data, &slots).is_ok());
    }

    #[test]
    fn missing_data() {
        let slots = vec![
            Slot {
                key: "key".to_string(),
                ..Default::default()
            },
            Slot {
                key: "key2".to_string(),
                ..Default::default()
            },
        ];

        let data = HashMap::from([("key", "value")])
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<String, String>>();

        assert!(validate_data(&data, &slots).is_err());
    }

    #[test]
    fn extra_data() {
        let slots = vec![Slot {
            key: "key".to_string(),
            ..Default::default()
        }];

        let data = HashMap::from([("key", "value"), ("key2", "value2")])
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<String, String>>();

        assert!(validate_data(&data, &slots).is_err());
    }

    #[test]
    fn non_string_type() {
        let slots = vec![
            Slot {
                key: "key".to_string(),
                r#type: SlotType::Number,
                ..Default::default()
            },
            Slot {
                key: "key2".to_string(),
                r#type: SlotType::Boolean,
                ..Default::default()
            },
        ];

        let data = HashMap::from([("key", "3.14"), ("key2", "true")])
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<String, String>>();

        assert!(validate_data(&data, &slots).is_ok());
    }

    #[test]
    fn wrong_type() {
        let slots = vec![Slot {
            key: "key".to_string(),
            r#type: SlotType::Number,
            ..Default::default()
        }];

        let data = HashMap::from([("key", "value")])
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<String, String>>();

        assert!(validate_data(&data, &slots).is_err());
    }
}
