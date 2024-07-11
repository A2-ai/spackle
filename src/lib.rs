use core::{
    config::{self},
    hook::{self, HookResult},
    template,
};
use std::{collections::HashMap, os::unix, path::PathBuf};

use users::User;
use util::copy;
use walkdir::WalkDir;

pub mod core;
pub mod util;

#[derive(Debug)]
pub enum Error {
    AlreadyExists(PathBuf),
    ConfigError(config::Error),
    CopyError(copy::Error),
    TemplateError(Box<dyn std::error::Error>),
    HookFailed(hook::Error),
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::AlreadyExists(dir) => write!(f, "Directory already exists: {}", dir.display()),
            Error::ConfigError(e) => write!(f, "Error loading config: {}", e),
            Error::TemplateError(e) => write!(f, "Error rendering template: {}", e),
            Error::CopyError(e) => write!(f, "Error copying files: {}", e),
            Error::HookFailed(e) => write!(f, "Hook failed: {}", e),
            Error::Other(e) => write!(f, "{}", e),
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
            Error::HookFailed(_) => None,
            Error::Other(_) => None,
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
    hook_data: &HashMap<String, bool>,
    run_as_user: Option<User>,
) -> Result<Vec<HookResult>, Error> {
    if out_dir.exists() {
        return Err(Error::AlreadyExists(out_dir.clone()));
    }

    let config = config::load(project_dir).map_err(Error::ConfigError)?;

    let mut slot_data = slot_data.clone();
    slot_data.insert(
        "project_name".to_string(),
        project_dir.file_name().unwrap().to_string_lossy().into(),
    );

    // Copy all non-template files to the output directory
    copy::copy(project_dir, &out_dir, &config.ignore, &slot_data).map_err(Error::CopyError)?;

    // Render template files to the output directory
    let results = template::fill(project_dir, out_dir, &slot_data)
        .map_err(|e| Error::TemplateError(e.into()))?;
    for result in results {
        if let Err(e) = result {
            return Err(Error::TemplateError(e.into()));
        }
    }

    // Change ownership of created directories to user
    if let Some(ref user) = run_as_user {
        let walker = WalkDir::new(&out_dir).into_iter().filter_map(|e| e.ok());
        for entry in walker {
            if let Err(e) = unix::fs::chown(
                entry.path(),
                Some(user.uid()),
                Some(user.primary_group_id()),
            ) {
                return Err(Error::Other(e.to_string()));
            }
        }
    }

    // Run post-template hooks in the output directory
    let results = hook::run_hooks(
        &config.hooks,
        out_dir,
        &slot_data,
        hook_data,
        run_as_user.clone(),
    )
    .map_err(Error::HookFailed)?;

    Ok(results)
}
