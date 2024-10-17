use std::{error::Error, process::exit, time::Instant};

use colored::Colorize;
use spackle::{
    slot,
    template::{self, ValidateError},
    Project,
};

pub fn run(project: &Project) {
    println!("🔍 Validating project configuration\n");

    let start_time = Instant::now();

    match template::validate(&project.path, &project.config.slots) {
        Ok(()) => {
            println!("  {}", "👌 Template files are valid".dimmed());
        }
        Err(e) => {
            match e {
                ValidateError::TeraError(e) => {
                    eprintln!(
                        "{}\n{}\n",
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
                            e.source().map(|e| e.to_string()).unwrap_or_default().red()
                        )
                    }
                }
            }

            print_elapsed_time(start_time);
            exit(1);
        }
    }

    match slot::validate(&project.config.slots) {
        Ok(()) => {
            println!("  {}\n", "👌 Slot data is valid".dimmed());
        }
        Err(e) => {
            eprintln!(
                "{}\n{}\n",
                "❌ Error validating slot configuration".bright_red(),
                e.to_string().red()
            );
            exit(1);
        }
    }

    print_elapsed_time(start_time);
}

fn print_elapsed_time(start_time: Instant) {
    println!(
        "  ✅ done {}",
        format!("in {:?}", start_time.elapsed()).dimmed()
    );
}
