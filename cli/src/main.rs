use clap::{command, Parser, Subcommand};
use colored::Colorize;
use spackle::Project;
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
        /// Assign data to a slot or hook
        #[arg(short, long)]
        data: Vec<String>,
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
    let project = match spackle::load_project(&project_dir) {
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
        Commands::Info {} => info::run(&project.config),
        Commands::Fill { data } => fill::run(data, &project, &cli.out, &cli),
    }
}

fn print_project_info(project: &Project) {
    println!(
        "📂 Using project {}\n  {}\n{}\n{}\n",
        project.get_name().bold(),
        project.dir.to_string_lossy().dimmed(),
        format!(
            "  🕳️  {} {}",
            project.config.slots.len(),
            if project.config.slots.len() == 1 {
                "slot"
            } else {
                "slots"
            }
        )
        .dimmed(),
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
}
