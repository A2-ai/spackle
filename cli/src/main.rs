use clap::{command, Parser, Subcommand};
use colored::Colorize;
use spackle::Project;
use std::{path::PathBuf, process::exit};
mod check;
mod fill;
mod info;
mod util;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The spackle project to use (either a directory or a single file). Defaults to the current directory.
    #[arg(short = 'p', long = "project", default_value = ".", global = true)]
    project_path: PathBuf,

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
        /// Assign data to a slot or hook
        #[arg(short, long)]
        data: Vec<String>,

        /// Whether to overwrite existing files
        #[arg(short = 'O', long)]
        overwrite: bool,

        /// The location the output should be written to. If the project is a single file, this is the output file. If the project is a directory, this is the output directory.
        #[arg(short = 'o', long = "out", global = true)]
        out_path: Option<PathBuf>,
    },
    /// Checks the validity of a spackle project
    Check,
}

fn main() {
    println!("{}\n", "🚰 spackle".truecolor(200, 200, 255));

    let cli = Cli::parse();

    let project = match spackle::load_project(&cli.project_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "❌ {}\n{}",
                "Error loading project config".bright_red(),
                e.to_string().red()
            );
            exit(1);
        }
    };

    print_project_info(&project);

    match &cli.command {
        Commands::Check => check::run(&project),
        Commands::Info => info::run(&project.config),
        Commands::Fill {
            data,
            overwrite,
            out_path,
        } => fill::run(data, overwrite, out_path, &project, &cli),
    }
}

fn print_project_info(project: &Project) {
    println!("📦 Using project {}\n", project.get_name().bold());

    println!(
        "  {}",
        format!("📁 {}", project.path.to_string_lossy()).dimmed()
    );

    println!(
        "{}",
        format!(
            "  🕳️  {} {}",
            project.config.slots.len(),
            if project.config.slots.len() == 1 {
                "slot"
            } else {
                "slots"
            }
        )
        .dimmed()
    );

    println!(
        "{}",
        format!(
            "  🪝  {} {}",
            project.config.hooks.len(),
            if project.config.hooks.len() == 1 {
                "hook"
            } else {
                "hooks"
            }
        )
        .dimmed()
    );
    println!();
}
