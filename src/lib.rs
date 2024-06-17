use core::{config, template};
use std::{collections::HashMap, path::PathBuf};

use util::copy;

pub mod core;
pub mod util;

#[derive(Debug)]
pub enum Error {
    AlreadyExists(PathBuf),
    ConfigError(config::Error),
    CopyError(copy::Error),
    TemplateError(Box<dyn std::error::Error>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::AlreadyExists(dir) => write!(f, "Directory already exists: {}", dir.display()),
            Error::ConfigError(e) => write!(f, "Error loading config: {}", e),
            Error::TemplateError(e) => write!(f, "Error rendering template: {}", e),
            Error::CopyError(e) => write!(f, "Error copying files: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::AlreadyExists(_) => None,
            Error::ConfigError(_) => None,
            Error::TemplateError(e) => Some(e.as_ref()),
            Error::CopyError(e) => Some(e),
        }
    }
}

/// Generates a filled directory from the specified spackle project.
///
/// out_dir is the path to what will become the filled directory
pub fn generate(
    project_dir: &PathBuf,
    data: &HashMap<String, String>,
    out_dir: &PathBuf,
) -> Result<(), Error> {
    if out_dir.exists() {
        return Err(Error::AlreadyExists(out_dir.clone()));
    }

    let config = config::load(project_dir).map_err(Error::ConfigError)?;

    copy::copy(project_dir, &out_dir, &config.ignore).map_err(Error::CopyError)?;

    // Fill the template, returning if any files failed to render
    let results =
        template::fill(project_dir, data, out_dir).map_err(|e| Error::TemplateError(e.into()))?;
    for result in results {
        if let Err(e) = result {
            return Err(Error::TemplateError(e.into()));
        }
    }

    Ok(())
}
