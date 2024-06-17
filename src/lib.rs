use core::{config, template};
use std::{collections::HashMap, path::PathBuf};

use util::copy;

pub mod core;
pub mod util;

/// Generates a filled directory from the specified spackle project.
///
/// out_dir is the directory to put the filled directory, and out_name is the name of the filled directory.
pub fn generate(
    project_dir: &PathBuf,
    data: &HashMap<String, String>,
    out_dir: &PathBuf,
    out_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let gen_dir = out_dir.join(out_name);

    let config = config::load(project_dir).map_err(|e| e.to_string())?;

    copy::copy(project_dir, &gen_dir, &config.ignore)?;

    // Fill the template, returning if any files failed to render
    let results = template::fill(project_dir, data, out_dir)?;
    for result in results {
        if let Err(e) = result {
            return Err(e.into());
        }
    }

    Ok(())
}
