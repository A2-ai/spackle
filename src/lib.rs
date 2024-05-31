pub mod core;

use core::{config, slot::Slot};
use std::path::PathBuf;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn info(project_dir: String) -> Result<Vec<Slot>, String> {
    // Load the config
    let config = match config::load(&PathBuf::from(project_dir)) {
        Ok(config) => config,
        Err(e) => {
            return Err(e.to_string());
        }
    };

    Ok(config.slots)
}
