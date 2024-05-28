use clap::{command, Parser, Subcommand};
use colored::{Color, Colorize};
use core::config;
use std::{borrow::BorrowMut, collections::HashMap, error::Error, fs, path::PathBuf};
use tera::Tera;

use crate::core::config::SlotType;

mod core;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The directory of the spackle project. Defaults to the current directory.
    #[arg(short = 'D', long, default_value = ".", global = true)]
    dir: PathBuf,

    /// The directory to render to. Defaults to 'out' within the project root.
    #[arg(short = 'o', long, default_value = "out", global = true)]
    out: PathBuf,

    /// Whether to run in verbose mode.
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Gets info on a spackle template including the required inputs
    /// and their descriptions.
    Info,
    /// Fills a spackle template using the provided slot data
    Fill {
        /// Provides a slot with data
        #[arg(short, long)]
        entries: Vec<String>,
    },
}

fn main() {
    println!("{}\n", "🚰 spackle".truecolor(200, 200, 255));

    let cli = Cli::parse();

    let project_dir = cli.dir;

    match &cli.command {
        Commands::Info {} => {
            // Check if the project directory is a spackle project
            if !project_dir.join("spackle.toml").exists() {
                eprintln!(
                    "{} {}",
                    "❌",
                    "Provided directory is not a spackle project. Valid projects must have a spackle.toml file.".bright_red()
                );
                std::process::exit(1);
            }

            println!("{}", "slots".truecolor(140, 200, 255).bold());

            // Load the config
            let config = match config::load(&project_dir) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("{} {}", "❌", e.to_string().bright_red());
                    std::process::exit(1);
                }
            };

            config.slots.into_iter().for_each(|slot| {
                println!("{}\n", slot);
            })
        }
        Commands::Fill { entries: data } => {
            // Check if the project directory is a spackle project
            if !project_dir.join("spackle.toml").exists() {
                eprintln!(
                    "{} {}",
                    "❌",
                    "Provided directory is not a spackle project. Valid projects must have a spackle.toml file.".bright_red()
                );
                std::process::exit(1);
            }

            // Load the config
            let config = match config::load(&project_dir) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("{} {}", "❌", e.to_string().bright_red());
                    std::process::exit(1);
                }
            };

            let data_entries = data
                .iter()
                .filter_map(|data| match data.split_once('=') {
                    Some((key, value)) => Some((key.to_string(), value.to_string())),
                    None => {
                        eprintln!(
                            "{} {}",
                            "❌",
                            "Invalid data argument, must be key=value. Skipping.".bright_red()
                        );
                        None
                    }
                })
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect::<HashMap<String, String>>();

            // All data elements must assign to a slot, and that argument must be for that data type
            let all_assigned = data_entries.clone().into_iter().all(|entry| {
                let slot = match config.slots.iter().find(|slot| slot.key == entry.0) {
                    Some(slot) => slot,
                    None => {
                        eprintln!(
                            "{} {} {}\n\n{}",
                            "❌ The key".bright_red(),
                            entry.0.truecolor(255, 100, 100).bold(),
                            "does not match a slot in the project. To see a list of available slots, run".bright_red(),
                            "spackle info".truecolor(180, 180, 240),
                        );
                        std::process::exit(1);
                    }
                };

                // Verify the data type by trying to parse it as the slot type
                let is_valid = match slot.r#type {
                    SlotType::String => entry.1.parse::<String>().is_ok(),
                    SlotType::Number => entry.1.parse::<f64>().is_ok(),
                    SlotType::Boolean => entry.1.parse::<bool>().is_ok(),
                };
                if !is_valid {
                    eprintln!(
                        "{} {}",
                        "❌",
                        "Data does not match the specified slot type".bright_red()
                    );
                    std::process::exit(1);
                }

                true
            });
            if !all_assigned {
                eprintln!(
                    "{} {}",
                    "❌",
                    "Some data arguments are not associated with a slot".bright_red()
                );
                std::process::exit(1);
            }

            // Load the templates
            let glob = project_dir.join("**").join("*.j2");
            let tera = match Tera::new(&glob.to_string_lossy()) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{} {}", "❌", e.to_string().bright_red());
                    std::process::exit(1);
                }
            };

            let context = match tera::Context::from_serialize(data_entries) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{} {}", "❌", e.to_string().bright_red());
                    std::process::exit(1);
                }
            };

            // print context
            if cli.verbose {
                println!("{:#?}\n", context);
            }

            // Render the template
            let template_names = tera.get_template_names().collect::<Vec<_>>();
            let num_templates = template_names.len();
            for (i, template_name) in template_names.iter().enumerate() {
                println!(
                    "{} rendering {}...\n",
                    ("📄 [".to_owned()
                        + &(i + 1).to_string()
                        + &"/"
                        + &num_templates.to_string()
                        + &"]")
                        .truecolor(128, 128, 128),
                    template_name.yellow()
                );

                let output = match tera.render(template_name, &context) {
                    Ok(o) => o,
                    Err(e) => {
                        eprintln!(
                            "{} {}\n{}",
                            "❌",
                            e.to_string().bright_red(),
                            e.source().unwrap().to_string().red()
                        );

                        "".to_string()
                    }
                };

                if cli.verbose {
                    println!("{}", output);
                }

                // Template the file name itself
                let mut tera = tera.clone();
                let template_name = match tera.render_str(template_name, &context) {
                    Ok(o) => o,
                    Err(e) => {
                        eprintln!(
                            "{}\n{}\n",
                            "❌ Failed to render file name".bright_red(),
                            e.source().unwrap().to_string().red()
                        );

                        continue;
                    }
                };

                let template_name = match template_name.strip_suffix(".j2") {
                    Some(name) => name,
                    None => return eprintln!("{}\n", "❌ Error with template name".bright_red()),
                };

                // Write the output
                let output_dir = project_dir.join("out").join(template_name);

                match fs::create_dir_all(output_dir.parent().unwrap()) {
                    Ok(_) => (),
                    Err(e) => match e.kind() {
                        std::io::ErrorKind::AlreadyExists => (),
                        _ => eprintln!(
                            "{}\n{}\n{}",
                            "❌ Error creating output directory".bright_red(),
                            e.to_string().bright_red(),
                            output_dir.to_string_lossy().bright_red()
                        ),
                    },
                }

                match fs::write(&output_dir, output) {
                    Ok(_) => (),
                    Err(e) => eprintln!(
                        "{}\n{}\n{}",
                        "❌ Error writing rendered file".bright_red(),
                        e.to_string().bright_red(),
                        output_dir.to_string_lossy().bright_red()
                    ),
                }
            }
        }
    }
}
