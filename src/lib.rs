use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
};

use tokio_stream::Stream;
use users::User;

pub mod config;
pub mod copy;
pub mod hook;
mod needs;
pub mod slot;
pub mod template;

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

// Loads the project from the specified directory or path and validates it
pub fn load_project(path: &PathBuf) -> Result<Project, config::Error> {
    let config = config::load(path)?;

    config.validate()?;

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
    /// Gets the name of the project or if one isn't specified, from the directory name
    pub fn get_name(&self) -> String {
        if let Some(name) = &self.config.name {
            return name.clone();
        }

        let path = match self.path.canonicalize() {
            Ok(path) => path,
            Err(_) => return "".to_string(),
        };

        return path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
    }

    pub fn validate(&self) -> Result<(), template::ValidateError> {
        template::validate(&self.path, &self.config.slots)
    }

    pub fn copy_files(
        &self,
        out_dir: &Path,
        data: &HashMap<String, String>,
    ) -> Result<copy::CopyResult, copy::Error> {
        let mut data = data.clone();
        data.insert("_project_name".to_string(), self.get_name());
        data.insert(
            "_output_name".to_string(),
            // TODO better handle unwrap
            out_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        );

        copy::copy(&self.path, out_dir, &self.config.ignore, &data)
    }

    pub fn render_templates(
        &self,
        out_dir: &Path,
        data: &HashMap<String, String>,
    ) -> Result<Vec<Result<template::RenderedFile, template::FileError>>, tera::Error> {
        let mut data = data.clone();
        data.insert("_project_name".to_string(), self.get_name());
        data.insert(
            "_output_name".to_string(),
            // TODO better handle unwrap
            out_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        );

        template::fill(&self.path, out_dir, &data)
    }

    /// Runs the hooks in the generated spackle project.
    ///
    /// out_dir is the path to the filled directory
    pub fn run_hooks_stream(
        &self,
        out_dir: &Path,
        slot_data: &HashMap<String, String>,
        run_as_user: Option<User>,
    ) -> Result<impl Stream<Item = hook::HookStreamResult>, RunHooksError> {
        let mut data = slot_data.clone();
        data.insert("_project_name".to_string(), self.get_name());
        data.insert(
            "_output_name".to_string(),
            // TODO better handle unwrap
            out_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        );

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

    /// Runs the hooks in the generated spackle project.
    ///
    /// out_dir is the path to the filled directory
    pub fn run_hooks(
        &self,
        out_dir: &Path,
        data: &HashMap<String, String>,
        run_as_user: Option<User>,
    ) -> Result<Vec<hook::HookResult>, hook::Error> {
        let mut data = data.clone();
        data.insert("_project_name".to_string(), self.get_name());
        data.insert(
            "_output_name".to_string(),
            // TODO better handle unwrap
            out_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        );

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
    use std::env;

    use crate::config::Config;

    use super::*;

    #[test]
    fn project_get_name_explicit() {
        let project = Project {
            config: Config {
                name: Some("some_name".to_string()),
                ..Default::default()
            },
            path: PathBuf::from("."),
        };

        assert_eq!(project.get_name(), "some_name");
    }

    #[test]
    fn project_get_name_inferred() {
        let project = Project {
            config: Config::default(),
            path: PathBuf::from("tests/data/templated"),
        };

        assert_eq!(project.get_name(), "templated");
    }

    #[test]
    fn project_get_name_cwd() {
        let cwd = env::current_dir().unwrap();

        env::set_current_dir(PathBuf::from("tests/data/templated")).unwrap();

        let project = Project {
            config: Config::default(),
            path: PathBuf::from("."),
        };

        assert_eq!(project.get_name(), "templated");

        // HACK find a better way to do this
        env::set_current_dir(cwd).unwrap();
    }
}
