use std::{error::Error, path::PathBuf, process::exit};

use colored::Colorize;
use spackle::core::{
    config::Config,
    template::{self, ValidateError},
};

pub fn run(project_dir: &PathBuf, config: &Config) {
    match template::validate(&project_dir, &config.slots) {
        Ok(()) => {
            println!("{}", "✅ Template files are valid".bright_green());
        }
        Err(e) => {
            match e {
                ValidateError::TeraError(e) => {
                    eprintln!(
                        "{}\n{}",
                        "❌ Error validating template files".bright_red(),
                        e.to_string().red()
                    );
                }
                ValidateError::RenderError(e) => {
                    for (templ, e) in e {
                        eprintln!(
                            "{}\n{}\n",
                            format!("❌ Template {} has errors", templ.bright_red().bold())
                                .bright_red(),
                            e.source()
                                .map(|e| e.to_string())
                                .unwrap_or_default()
                                .bright_red()
                                .dimmed()
                        )
                    }
                }
            }

            exit(1);
        }
    }
}
