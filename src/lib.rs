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
pub mod slot;
pub mod template;

pub struct Project {
    pub config: config::Config,
    pub dir: PathBuf,
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

// Loads the config from the project directory and validates it
pub fn load_project(dir: &PathBuf) -> Result<Project, config::Error> {
    let config = config::load(dir)?;

    config.validate()?;

    Ok(Project {
        config,
        dir: dir.to_owned(),
    })
}

impl Project {
    /// Gets the name of the project or if one isn't specified, from the directory name
    pub fn get_name(&self) -> String {
        if let Some(name) = &self.config.name {
            return name.clone();
        }

        let path = match self.dir.canonicalize() {
            Ok(path) => path,
            Err(_) => return "".to_string(),
        };

        return path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
    }

    pub fn validate(&self) -> Result<(), template::ValidateError> {
        template::validate(&self.dir, &self.config.slots)
    }

    pub fn copy_files(
        &self,
        out_dir: &Path,
        slot_data: &HashMap<String, String>,
    ) -> Result<copy::CopyResult, copy::Error> {
        let mut slot_data = slot_data.clone();
        slot_data.insert("project_name".to_string(), self.get_name());

        copy::copy(&self.dir, out_dir, &self.config.ignore, &slot_data)
    }

    pub fn render_templates(
        &self,
        out_dir: &Path,
        slot_data: &HashMap<String, String>,
    ) -> Result<Vec<Result<template::RenderedFile, template::FileError>>, tera::Error> {
        let mut slot_data = slot_data.clone();
        slot_data.insert("project_name".to_string(), self.get_name());

        template::fill(&self.dir, out_dir, &slot_data)
    }

    /// Runs the hooks in the generated spackle project.
    ///
    /// out_dir is the path to the filled directory
    pub fn run_hooks_stream(
        &self,
        out_dir: &Path,
        slot_data: &HashMap<String, String>,
        hook_data: &HashMap<String, bool>,
        run_as_user: Option<User>,
    ) -> Result<impl Stream<Item = hook::HookStreamResult>, RunHooksError> {
        let mut slot_data = slot_data.clone();
        slot_data.insert("project_name".to_string(), self.get_name());

        let result = hook::run_hooks_stream(
            &self.config.hooks,
            out_dir.to_owned(),
            &self.config.slots,
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
        &self,
        out_dir: &Path,
        slot_data: &HashMap<String, String>,
        hook_data: &HashMap<String, bool>,
        run_as_user: Option<User>,
    ) -> Result<Vec<hook::HookResult>, hook::Error> {
        let mut slot_data = slot_data.clone();
        slot_data.insert("project_name".to_string(), self.get_name());

        let result = hook::run_hooks(
            &self.config.hooks,
            out_dir.to_owned(),
            &self.config.slots,
            &slot_data,
            hook_data,
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
            dir: PathBuf::from("."),
        };

        assert_eq!(project.get_name(), "some_name");
    }

    #[test]
    fn project_get_name_inferred() {
        let project = Project {
            config: Config::default(),
            dir: PathBuf::from("tests/data/templated"),
        };

        assert_eq!(project.get_name(), "templated");
    }

    #[test]
    fn project_get_name_cwd() {
        let cwd = env::current_dir().unwrap();

        env::set_current_dir(PathBuf::from("tests/data/templated")).unwrap();

        let project = Project {
            config: Config::default(),
            dir: PathBuf::from("."),
        };

        assert_eq!(project.get_name(), "templated");

        // HACK find a better way to do this
        env::set_current_dir(cwd).unwrap();
    }
}
