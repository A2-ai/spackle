use core::{config, template};
use std::{collections::HashMap, path::PathBuf};

use util::copy;

pub mod core;
pub mod util;

/// Generates a filled directory from the specified spackle project.
///
/// out_dir is the path to what will become the filled directory
pub fn generate(
    project_dir: &PathBuf,
    data: &HashMap<String, String>,
    out_dir: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = config::load(project_dir).map_err(|e| e.to_string())?;

    copy::copy(project_dir, &out_dir, &config.ignore)?;

    // Fill the template, returning if any files failed to render
    let results = template::fill(project_dir, data, out_dir)?;
    for result in results {
        if let Err(e) = result {
            return Err(e.into());
        }
    }

    Ok(())
}
