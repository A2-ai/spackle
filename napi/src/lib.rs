use spackle::core::config;
use spackle::core::slot;
use std::path::PathBuf;

use napi::bindgen_prelude::*;
use napi_derive::napi;

// TODO currently can't derive in core since building bin will trigger linker errors since not dynamic
#[napi]
pub struct Slot {
    pub key: String,
    pub r#type: SlotType,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[napi]
impl From<spackle::core::slot::Slot> for Slot {
    fn from(slot: spackle::core::slot::Slot) -> Self {
        Self {
            key: slot.key,
            r#type: slot.r#type.into(),
            name: slot.name,
            description: slot.description,
        }
    }
}

#[napi]
pub enum SlotType {
    String,
    Number,
    Boolean,
}

impl From<slot::SlotType> for SlotType {
    fn from(slot_type: slot::SlotType) -> Self {
        match slot_type {
            slot::SlotType::String => Self::String,
            slot::SlotType::Number => Self::Number,
            slot::SlotType::Boolean => Self::Boolean,
        }
    }
}

#[napi]
pub fn info(project_dir: String) -> Result<Vec<Slot>, String> {
    // Load the config
    let config = match config::load(&PathBuf::from(project_dir)) {
        Ok(config) => config,
        Err(e) => return Err(napi::Error::new("Error loading config".to_string(), e)),
    };

    Ok(config.slots.into_iter().map(|s| s.into()).collect())
}

#[napi]
pub fn fibonacci(n: u32) -> u32 {
    match n {
        1 | 2 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}
