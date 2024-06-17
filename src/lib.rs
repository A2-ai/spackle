use core::{config, template};
use std::{collections::HashMap, path::PathBuf};

use util::copy;

pub mod core;
pub mod util;

pub fn generate(
    project_dir: &PathBuf,
    data: &HashMap<String, String>,
    out_dir: &PathBuf,
    out_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let gen_dir = out_dir.join(out_name);

    // Load the project
    let config = config::load(project_dir).map_err(|e| e.to_string())?;

    copy::copy(project_dir, &gen_dir, &config.ignore)?;

    template::fill(project_dir, data, out_dir)?;

    Ok(())
}
