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

pub struct Project {
    pub config: config::Config,
    pub dir: PathBuf,
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
    // TODO add top-level config for forcing project name
    /// Gets the name of the project from the directory name
    pub fn get_name(&self) -> String {
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

    /// Generates a filled directory from the project
    ///
    /// out_dir is the path to what will become the filled directory
    pub fn generate(
        self,
        out_dir: &PathBuf,
        slot_data: &HashMap<String, String>,
    ) -> Result<Vec<RenderedFile>, GenerateError> {
        if out_dir.exists() {
            return Err(GenerateError::AlreadyExists(out_dir.clone()));
        }

        let mut slot_data = slot_data.clone();
        slot_data.insert(
            "project_name".to_string(),
            self.dir.file_name().unwrap().to_string_lossy().into(),
        );

        // Copy all non-template files to the output directory
        copy::copy(&self.dir, &out_dir, &self.config.ignore, &slot_data)
            .map_err(GenerateError::CopyError)?;

        // Render template files to the output directory
        // TODO improve returned error type here
        let results = template::fill(&self.dir, &out_dir, &slot_data)
            .map_err(|_| GenerateError::TemplateError)?;

        if results.iter().any(|r| r.is_err()) {
            return Err(GenerateError::TemplateError);
        }

        Ok(results.into_iter().filter_map(|r| r.ok()).collect())
    }

    /// Runs the hooks in the generated spackle project.
    ///
    /// out_dir is the path to the filled directory
    pub fn run_hooks_stream(
        self,
        out_dir: PathBuf,
        slot_data: &HashMap<String, String>,
        hook_data: &HashMap<String, bool>,
        run_as_user: Option<User>,
    ) -> Result<impl Stream<Item = HookStreamResult>, RunHooksError> {
        let result = hook::run_hooks_stream(
            &self.config.hooks,
            out_dir,
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
        self,
        out_dir: &PathBuf,
        slot_data: &HashMap<String, String>,
        hook_data: &HashMap<String, bool>,
        run_as_user: Option<User>,
    ) -> Result<Vec<HookResult>, RunHooksError> {
        let result = hook::run_hooks(
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
