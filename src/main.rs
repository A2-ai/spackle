use clap::{command, Parser, Subcommand};
use colored::Colorize;
use core::{config, validate};
use std::{collections::HashMap, path::PathBuf, time::Instant};

use crate::core::fill;

mod core;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The directory of the spackle project. Defaults to the current directory.
    #[arg(short = 'D', long, default_value = ".", global = true)]
    dir: PathBuf,

    /// The directory to render to. Defaults to 'render' within the project root.
    #[arg(short = 'o', long, default_value = "render", global = true)]
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

    // Check if the project directory is a spackle project
    if !project_dir.join("spackle.toml").exists() {
        eprintln!(
            "{}\n{}",
            "❌ Provided directory is not a spackle project".bright_red(),
            "Valid projects must have a spackle.toml file.".red()
        );
        std::process::exit(1);
    }

    // Load the config
    let config = match config::load(&project_dir) {
        Ok(config) => config,
        Err(e) => {
            eprintln!(
                "{} {}",
                "❌ Error loading project config",
                e.to_string().bright_red()
            );
            std::process::exit(1);
        }
    };

    println!(
        "📂 {} {}\n",
        "Using project",
        project_dir.to_string_lossy().bold()
    );

    match &cli.command {
        Commands::Info {} => {
            println!("{}", "slots".truecolor(140, 200, 255).bold());

            config.slots.into_iter().for_each(|slot| {
                println!("{}\n", slot);
            });
        }
        Commands::Fill { entries: data } => {
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

            match validate::validate(&data_entries, config.slots) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!(
                        "❌ {}\n{}",
                        "Error validating supplied data".bright_red(),
                        e.to_string().red()
                    );

                    std::process::exit(1);
                }
            }

            let start_time = Instant::now();

            match fill::fill(&project_dir, data_entries, &PathBuf::from(&cli.out)) {
                Ok(r) => {
                    println!(
                        "{} {} {} {} {}\n",
                        "🏁 Processed",
                        r.len(),
                        "files",
                        "in".dimmed(),
                        format!("{:?}", start_time.elapsed()).dimmed()
                    );

                    for result in r {
                        match result {
                            Ok(f) => {
                                if cli.verbose {
                                    println!(
                                        "📄 Processed {} {} {}\n",
                                        f.path.to_string_lossy().bold(),
                                        "in".dimmed(),
                                        format!("{:?}", f.elapsed).dimmed()
                                    );

                                    println!(
                                        "{}\n",
                                        f.contents
                                            .lines()
                                            .map(|line| format!("  {}", line))
                                            .collect::<Vec<String>>()
                                            .join("\n")
                                    );
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "{} {}\n{}\n",
                                    "⚠️ Could not process file".bright_yellow(),
                                    e.file.bright_yellow().bold(),
                                    format!("{}\n{}", e.kind, e.source.source().unwrap())
                                        .yellow()
                                        .dimmed(),
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "❌ {}\n{}",
                        "Could not fill project".bright_red(),
                        e.to_string().red(),
                    );
                }
            }
        }
    }
}
