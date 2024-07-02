use clap::{command, Parser, Subcommand};
use colored::Colorize;
use spackle::core::config;
use std::{path::PathBuf, process::exit};

mod check;
mod fill;
mod info;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The directory of the spackle project. Defaults to the current directory.
    #[arg(short = 'D', long, default_value = ".", global = true)]
    dir: PathBuf,

    /// The directory to render to. Defaults to 'render' within the current directory. Cannot be the same as the project directory.
    #[arg(short = 'o', long, default_value = "render", global = true)]
    out: PathBuf,

    /// Whether to run in verbose mode.
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Gets info on a spackle project including the required inputs
    /// and their descriptions.
    Info,
    /// Fills a spackle project using the provided data
    Fill {
        /// Assign a given slot a value
        #[arg(short, long)]
        slot: Vec<String>,

        /// Toggle a given hook on or off
        #[arg(short = 'H', long)]
        hook: Vec<String>,
    },
    /// Checks the validity of a spackle project
    Check,
}

fn main() {
    println!("{}\n", "🚰 spackle".truecolor(200, 200, 255));

    let cli = Cli::parse();

    // Ensure the output directory is not the same as the project directory
    if cli.out == cli.dir {
        eprintln!(
            "{}\n{}",
            "❌ Output directory cannot be the same as project directory".bright_red(),
            "Please choose a different output directory.".red()
        );
        exit(2);
    }

    let project_dir = cli.dir.clone();

    // Check if the project directory is a spackle project
    if !project_dir.join("spackle.toml").exists() {
        eprintln!(
            "{}\n{}",
            "❌ Provided directory is not a spackle project".bright_red(),
            "Valid projects must have a spackle.toml file.".red()
        );
        exit(1);
    }

    // Load the config
    let config = match config::load(&project_dir) {
        Ok(config) => config,
        Err(e) => {
            eprintln!(
                "❌ {}\n{}",
                "Error loading project config".bright_red(),
                e.to_string().red()
            );
            exit(1);
        }
    };

    println!(
        "📂 {} {} {}\n",
        "Using project",
        project_dir.to_string_lossy().bold(),
        format!("with {} {}", config.slots.len(), "slots").dimmed()
    );

    match &cli.command {
        Commands::Check => check::run(&project_dir, &config),
        Commands::Info {} => info::run(&config),
        Commands::Fill { slot, hook } => {
            fill::run(slot, hook, &project_dir, &cli.out, &config, &cli)
        }
    }
}
