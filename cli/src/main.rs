use clap::{command, Parser, Subcommand};
use colored::Colorize;
use fronma::engines::Toml;
use fronma::parser::parse_with_engine;
use spackle::core::config::{self, Config};
use std::fs;
use std::{path::PathBuf, process::exit};
mod check;
mod fill;
mod info;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short = 'f', long, default_value = None, global = true)]
    file: Option<PathBuf>,

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
    if cli.file.is_none() && cli.out == cli.dir {
        eprintln!(
            "{}\n{}",
            "❌ Output directory cannot be the same as project directory".bright_red(),
            "Please choose a different output directory.".red()
        );
        exit(2);
    }

    let project_dir = cli.dir.clone();
    let config = if cli.file.is_none() {
        match config::load(&project_dir) {
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
        let file_contents = fs::read_to_string(cli.file.clone().unwrap()).unwrap();
        // TODO: don't duplicate this lol
        parse_with_engine::<spackle::core::config::Config, Toml>(&file_contents)
            .unwrap()
            .headers
        // parse config off file frontmatter
    };
    // Check if the project directory is a spackle project
    if cli.file.is_none() && !project_dir.join("spackle.toml").exists() {
        eprintln!(
            "{}\n{}",
            "❌ Provided directory is not a spackle project".bright_red(),
            "Valid projects must have a spackle.toml file.".red()
        );
        exit(1);
    }

    // Load the config
    if cli.file.is_none() {
        print_project_info(&project_dir, &config);
    }

    match &cli.command {
        Commands::Check => check::run(&project_dir, &config),
        Commands::Info {} => info::run(&config),
        Commands::Fill { slot, hook } => {
            if cli.file.is_none() {
                fill::run(slot, hook, &project_dir, &cli.out, &config, &cli)
            } else {
                // TODO: refactor - at the moment this is also duplicated in the config parsing logic
                // where its read in prior. just doing this quick and dirty now where not dealing with
                // the scoping rules/conditions of file vs dir
                let file_contents = fs::read_to_string(cli.file.clone().unwrap()).unwrap();
                let result =
                    parse_with_engine::<spackle::core::config::Config, Toml>(&file_contents)
                        .unwrap();
                let res = fill::run_file(result.body.to_string(), slot, config.slots);
                println!("{}", res.unwrap());
            }
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
