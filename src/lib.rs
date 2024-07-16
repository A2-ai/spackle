use core::{
    config, copy,
    template::{self, RenderedFile},
};
use std::{collections::HashMap, fmt::Display, path::PathBuf};

pub mod core;

#[derive(Debug)]
pub enum GenerateError {
    AlreadyExists(PathBuf),
    BadConfig(config::Error),
    CopyError(copy::Error),
    TemplateError,
}

impl Display for GenerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerateError::AlreadyExists(dir) => {
                write!(f, "Directory already exists: {}", dir.display())
            }
            GenerateError::BadConfig(e) => write!(f, "Error loading config: {}", e),
            GenerateError::TemplateError => write!(f, "Error rendering template"),
            GenerateError::CopyError(e) => write!(f, "Error copying files: {}", e),
        }
    }
}

impl std::error::Error for GenerateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GenerateError::AlreadyExists(_) => None,
            GenerateError::BadConfig(_) => None,
            GenerateError::TemplateError => None,
            GenerateError::CopyError(e) => Some(e),
        }
    }
}

/// Generates a filled directory from the specified spackle project.
///
/// out_dir is the path to what will become the filled directory
pub fn generate(
    project_dir: &PathBuf,
    out_dir: &PathBuf,
    slot_data: &HashMap<String, String>,
) -> Result<Vec<RenderedFile>, GenerateError> {
    if out_dir.exists() {
        return Err(GenerateError::AlreadyExists(out_dir.clone()));
    }

    let config = config::load(project_dir).map_err(GenerateError::BadConfig)?;

    let mut slot_data = slot_data.clone();
    slot_data.insert(
        "project_name".to_string(),
        project_dir.file_name().unwrap().to_string_lossy().into(),
    );

    // Copy all non-template files to the output directory
    copy::copy(project_dir, &out_dir, &config.ignore, &slot_data)
        .map_err(GenerateError::CopyError)?;

    // Render template files to the output directory
    // TODO improve returned error type here
    let results = template::fill(project_dir, out_dir, &slot_data)
        .map_err(|_| GenerateError::TemplateError)?;

    if results.iter().any(|r| r.is_err()) {
        return Err(GenerateError::TemplateError);
    }

    Ok(results.into_iter().filter_map(|r| r.ok()).collect())
}
