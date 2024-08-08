use core::{
    config::{self},
    copy,
    hook::{self, HookResult, HookStreamResult},
    template::{self, RenderedFile},
};
use std::{collections::HashMap, fmt::Display, path::PathBuf};

use tokio_stream::Stream;
use users::User;

pub mod core;

// Loads the config from the project directory and validates it
pub fn load(project_dir: &PathBuf) -> Result<config::Config, config::Error> {
    let config = config::load(project_dir)?;

    config.validate()?;

    Ok(config)
}

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

#[derive(Debug)]
pub enum RunHooksError {
    BadConfig(config::Error),
    HookError(hook::Error),
}

impl Display for RunHooksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunHooksError::BadConfig(e) => write!(f, "Error loading config: {}", e),
            RunHooksError::HookError(e) => write!(f, "Error running hook: {}", e),
        }
    }
}

/// Runs the hooks in the generated spackle project.
///
/// out_dir is the path to the filled directory
pub fn run_hooks_stream(
    project_dir: &PathBuf,
    out_dir: PathBuf,
    slot_data: &HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
    run_as_user: Option<User>,
) -> Result<impl Stream<Item = HookStreamResult>, RunHooksError> {
    let config = config::load(project_dir).map_err(RunHooksError::BadConfig)?;

    let result = hook::run_hooks_stream(
        &config.hooks,
        out_dir,
        &slot_data,
        hook_data,
        run_as_user.clone(),
    )
    .map_err(RunHooksError::HookError)?;

    Ok(result)
}

/// Runs the hooks in the generated spackle project.
///
/// out_dir is the path to the filled directory
pub fn run_hooks(
    project_dir: &PathBuf,
    out_dir: &PathBuf,
    slot_data: &HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
    run_as_user: Option<User>,
) -> Result<Vec<HookResult>, RunHooksError> {
    let config = config::load(project_dir).map_err(RunHooksError::BadConfig)?;

    let result = hook::run_hooks(
        &config.hooks,
        out_dir.to_owned(),
        &slot_data,
        hook_data,
        run_as_user.clone(),
    )
    .map_err(RunHooksError::HookError)?;

    Ok(result)
}
