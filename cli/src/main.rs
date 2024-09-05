use clap::{command, Parser, Subcommand};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Input};
use spackle::core::config::{self, Config};
use std::{path::PathBuf, process::exit};
mod check;
mod fill;
mod info;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The directory of the spackle project or the single file to render. Defaults to the current directory.
    #[arg(short = 'p', long = "project", default_value = ".", global = true)]
    project_path: PathBuf,

    /// The path to render to. If the project is a single file, this is the output file. If the project is a directory, this is the output directory.
    #[arg(short = 'o', long = "out", global = true)]
    out_dir: Option<PathBuf>,

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

    let out_path = match &cli.out_dir {
        Some(path) => path,
        None => &Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter the output path")
            .interact_text()
            .map(|p: String| PathBuf::from(p))
            .unwrap_or_else(|e| {
                eprintln!("❌ {}", e.to_string().red());
                exit(1);
            }),
    };

    // Ensure the output path doesn't exist
    if out_path.exists() {
        eprintln!(
            "{}\n{}",
            "❌ Output path already exists".bright_red(),
            "Please choose a different output path.".red()
        );
        exit(2);
    }

    // Load the config
    // this can either be a directory or a single file
    let config = if cli.project_path.is_dir() {
        if !cli.project_path.join("spackle.toml").exists() {
            eprintln!(
                "{}\n{}",
                "❌ Provided directory is not a spackle project".bright_red(),
                "Valid projects must have a spackle.toml file.".red()
            );
            exit(1);
        }

        match config::load_dir(&cli.project_path) {
            Ok(config) => config,
            Err(e) => {
                eprintln!(
                    "❌ {}\n{}",
                    "Error loading project config".bright_red(),
                    e.to_string().red()
                );
                exit(1);
            }
        }
    } else {
        match config::load_file(&cli.project_path) {
            Ok(config) => config,
            Err(e) => {
                eprintln!(
                    "❌ {}\n{}",
                    "Error loading project file".bright_red(),
                    e.to_string().red()
                );
                exit(1);
            }
        }
    };

    if cli.project_path.is_dir() {
        print_project_info(&cli.project_path, &config);
    } else {
        println!(
            "📄 Using project file {}\n",
            cli.project_path.to_string_lossy().bold()
        );
    }

    match &cli.command {
        Commands::Check => check::run(&cli.project_path, &config),
        Commands::Info {} => info::run(&config),
        Commands::Fill { slot, hook } => {
            fill::run(slot, hook, &cli.project_path, &out_path, &config, &cli)
        }
    }
}

fn print_project_info(project_dir: &PathBuf, config: &Config) {
    println!(
        "📂 {} {}\n{}\n{}\n",
        "Using project",
        project_dir.to_string_lossy().bold(),
        format!(
            "  🕳️  {} {}",
            config.slots.len(),
            if config.slots.len() == 1 {
                "slot"
            } else {
                "slots"
            }
        )
        .dimmed(),
        format!(
            "  🪝  {} {}",
            config.hooks.len(),
            if config.hooks.len() == 1 {
                "hook"
            } else {
                "hooks"
            }
        )
        .dimmed()
    );
}
