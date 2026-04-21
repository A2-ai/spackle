use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
#[cfg(not(target_arch = "wasm32"))]
use std::fmt::Display;

use template::RenderedFile;
use thiserror::Error;

#[cfg(not(target_arch = "wasm32"))]
use tokio_stream::Stream;
#[cfg(not(target_arch = "wasm32"))]
use users::User;

pub mod config;
pub mod copy;
pub mod fs;
pub mod hook;
pub mod needs;
pub mod slot;
pub mod template;

/// Pure error-kind mapping helpers for the `SpackleFs` contract.
/// Native-testable (no js-sys); `wasm_fs.rs` uses it via the `wasm32`
/// cfg-gated module.
#[cfg(feature = "wasm")]
pub mod wasm_fs_kind;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub mod wasm_fs;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub mod wasm;

#[derive(Error, Debug)]
pub enum LoadError {
    #[error("Error loading config from {path}: {error}")]
    ConfigError { path: PathBuf, error: config::Error },
}

#[derive(Error, Debug)]
pub enum CheckError {
    #[error("Error validating template files: {0}")]
    TemplateError(template::ValidateError),
    #[error("Error validating slot configuration: {0}")]
    SlotError(slot::Error),
}

#[derive(Error, Debug)]
pub enum GenerateError {
    #[error("The output directory already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("Error loading config: {0}")]
    BadConfig(config::Error),
    #[error("Error copying files: {0}")]
    CopyError(copy::Error),
    #[error("Error rendering templates: {0}")]
    TemplateError(#[from] tera::Error),
    #[error("Error rendering file: {0}")]
    FileError(#[from] template::FileError),
}

/// Derive a human-readable output name from `out_dir`.
///
/// Returns the final path component, falling back to `"project"` for
/// paths that have none (e.g. `/`). Does NOT canonicalize — under
/// wasm32 (`JsFs` path) canonicalize would require host-side fs access
/// that we explicitly avoid in the fs-bridge architecture. Callers
/// that want canonical paths should resolve them before passing in.
pub fn get_output_name(out_dir: &Path) -> String {
    out_dir
        .file_name()
        .unwrap_or("project".as_ref())
        .to_string_lossy()
        .to_string()
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub enum RunHooksError {
    BadConfig(config::Error),
    HookError(hook::Error),
}

#[cfg(not(target_arch = "wasm32"))]
impl Display for RunHooksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunHooksError::BadConfig(e) => write!(f, "Error loading config: {}", e),
            RunHooksError::HookError(e) => write!(f, "Error running hook: {}", e),
        }
    }
}

pub fn load_project<F: fs::FileSystem>(
    fs: &F,
    path: &PathBuf,
) -> Result<Project, LoadError> {
    let config = config::load(fs, path).map_err(|e| LoadError::ConfigError {
        path: path.to_owned(),
        error: e,
    })?;

    config.validate().map_err(|e| LoadError::ConfigError {
        path: path.to_owned(),
        error: e,
    })?;

    Ok(Project {
        config,
        path: path.to_owned(),
    })
}

pub struct Project {
    pub config: config::Config,
    pub path: PathBuf,
}

impl Project {
    /// Project name — `config.name` if set, otherwise the directory's
    /// file stem. Does NOT canonicalize; see `get_output_name` for the
    /// same rationale (no host fs access from wasm32).
    pub fn get_name(&self) -> String {
        if let Some(name) = &self.config.name {
            return name.clone();
        }

        self.path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    pub fn check<F: fs::FileSystem>(&self, fs: &F) -> Result<(), CheckError> {
        if let Err(e) = template::validate(fs, &self.path, &self.config.slots) {
            return Err(CheckError::TemplateError(e));
        }

        if let Err(e) = slot::validate(&self.config.slots) {
            return Err(CheckError::SlotError(e));
        }

        Ok(())
    }

    /// Generates a filled directory from the specified spackle project.
    ///
    /// out_dir is the path to what will become the filled directory
    pub fn generate<F: fs::FileSystem>(
        &self,
        fs: &F,
        project_dir: &PathBuf,
        out_dir: &PathBuf,
        slot_data: &HashMap<String, String>,
    ) -> Result<Vec<RenderedFile>, GenerateError> {
        if fs.exists(out_dir) {
            return Err(GenerateError::AlreadyExists(out_dir.clone()));
        }

        let config = config::load_dir(fs, project_dir).map_err(GenerateError::BadConfig)?;

        let mut slot_data = slot_data.clone();
        slot_data.insert("_project_name".to_string(), self.get_name());
        slot_data.insert("_output_name".to_string(), get_output_name(out_dir));

        // Copy all non-template files to the output directory
        copy::copy(fs, project_dir, &out_dir, &config.ignore, &slot_data)
            .map_err(GenerateError::CopyError)?;

        // Render template files to the output directory
        let results = template::fill(fs, project_dir, out_dir, &slot_data)
            .map_err(GenerateError::TemplateError)?;

        // Split vector into vector of rendered files and vector of errors
        let mut okay_results = Vec::new();

        for result in results {
            match result {
                Ok(rendered_file) => okay_results.push(rendered_file),
                Err(error) => return Err(GenerateError::FileError(error)),
            }
        }

        Ok(okay_results)
    }

    pub fn copy_files<F: fs::FileSystem>(
        &self,
        fs: &F,
        out_dir: &Path,
        data: &HashMap<String, String>,
    ) -> Result<copy::CopyResult, copy::Error> {
        let mut data = data.clone();
        data.insert("_project_name".to_string(), self.get_name());
        data.insert("_output_name".to_string(), get_output_name(out_dir));

        copy::copy(fs, &self.path, out_dir, &self.config.ignore, &data)
    }

    pub fn render_templates<F: fs::FileSystem>(
        &self,
        fs: &F,
        out_dir: &Path,
        data: &HashMap<String, String>,
    ) -> Result<Vec<Result<template::RenderedFile, template::FileError>>, tera::Error> {
        let mut data = data.clone();
        data.insert("_project_name".to_string(), self.get_name());
        data.insert("_output_name".to_string(), get_output_name(out_dir));

        template::fill(fs, &self.path, out_dir, &data)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_hooks_stream(
        &self,
        out_dir: &Path,
        data: &HashMap<String, String>,
        run_as_user: Option<User>,
    ) -> Result<impl Stream<Item = hook::HookStreamResult>, RunHooksError> {
        let mut data = data.clone();
        data.insert("_project_name".to_string(), self.get_name());
        data.insert("_output_name".to_string(), get_output_name(out_dir));

        let result = hook::run_hooks_stream(
            out_dir.to_owned(),
            &self.config.hooks,
            &self.config.slots,
            &data,
            run_as_user.clone(),
        )
        .map_err(RunHooksError::HookError)?;

        Ok(result)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_hooks(
        &self,
        out_dir: &Path,
        data: &HashMap<String, String>,
        run_as_user: Option<User>,
    ) -> Result<Vec<hook::HookResult>, hook::Error> {
        let mut data = data.clone();
        data.insert("_project_name".to_string(), self.get_name());
        data.insert("_output_name".to_string(), get_output_name(out_dir));

        let result = hook::run_hooks(
            &self.config.hooks,
            out_dir,
            &self.config.slots,
            &data,
            run_as_user.clone(),
        )?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_output_name() {
        let path = Path::new("/path/to/output.name");
        assert_eq!(get_output_name(path), "output.name");
    }

    use crate::fs::StdFs;

    #[test]
    fn test_check_pass() {
        let fs = StdFs::new();
        let project = load_project(&fs, &PathBuf::from("tests/data/proj2")).unwrap();
        let result = project.check(&fs);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_load_config_error() {
        let fs = StdFs::new();
        let path = PathBuf::from("tests/data/bad_config");
        let result = load_project(&fs, &path);
        assert!(
            result.is_err_and(|e| matches!(e, LoadError::ConfigError { path: p, .. } if p == path))
        );
    }

    #[test]
    fn test_check_slot_error() {
        let fs = StdFs::new();
        let project = load_project(&fs, &PathBuf::from("tests/data/bad_default_slot_val")).unwrap();
        let result = project.check(&fs);
        assert!(result.is_err_and(|e| matches!(e, CheckError::SlotError(_))));
    }

    #[test]
    fn test_check_template_error() {
        let fs = StdFs::new();
        let project = load_project(&fs, &PathBuf::from("tests/data/bad_template")).unwrap();
        let result = project.check(&fs);
        assert!(result.is_err_and(|e| matches!(e, CheckError::TemplateError(_))));
    }
}
