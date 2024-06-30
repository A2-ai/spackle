use clap::{command, Parser, Subcommand};
use colored::Colorize;
use spackle::{
    core::{
        config, hook, slot,
        template::{self, ValidateError},
    },
    util::copy,
};
use tera::Context;
use std::{collections::HashMap, error::Error, path::PathBuf, process::exit, time::Instant};

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
        /// Provides a slot with data
        #[arg(short, long)]
        entries: Vec<String>,
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

    let project_dir = cli.dir;

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
        Commands::Check => match template::validate(&project_dir, &config.slots) {
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
        },
        Commands::Info {} => {
            println!("{}", "slots".truecolor(140, 200, 255).bold());

            (&config.slots).into_iter().for_each(|slot| {
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

            match slot::validate_data(&data_entries, config.slots) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!(
                        "❌ {}\n{}",
                        "Error validating supplied data".bright_red(),
                        e.to_string().red()
                    );

                    exit(1);
                }
            }

            let start_time = Instant::now();

            // CR(devin): when looking at the below code, this likely should be pushed
            // into the spackle lib itself, there are too many implementation details
            // in the CLi that would also need to be replicated in any api/other client
            // when by the time you get to actually rendering the template
            // the fact this is touching like a util related module shows its
            // breaking the ideal implementation boundaries.
            
            // TODO: refactor the data_entries and context boundaries after considering
            // the api surface area
            let mut context = Context::new();
            data_entries.iter().for_each(|(key, value)| {
                context.insert(key, value);
            });
            match copy::copy(&project_dir, &cli.out, &config.ignore, &context) {
                Ok(r) => {
                    println!(
                        "{} {} {} {}",
                        "🖨️  Copied",
                        r.copied_count,
                        "files",
                        format!("in {:?}", start_time.elapsed()).dimmed()
                    );

                    if r.skipped_count > 0 {
                        println!(
                            "{}",
                            format!(
                                "{} {} {}",
                                "  Ignored", r.skipped_count, "files/directories"
                            )
                            .to_string()
                            .dimmed()
                        );
                    }

                    println!();
                }
                Err(e) => {
                    std::fs::remove_dir_all(&cli.out).unwrap();

                    eprintln!(
                        "❌ {}\n{}\n{}",
                        "Could not copy project".bright_red(),
                        e.path.to_string_lossy().red(),
                        e.to_string().red(),
                    );

                    exit(1);
                }
            }

            let start_time = Instant::now();

            match template::fill(&project_dir, &data_entries, &PathBuf::from(&cli.out)) {
                Ok(r) => {
                    println!(
                        "{} {} {} {} {}\n",
                        "⛽ Processed",
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
                                        .bright_yellow()
                                        .dimmed(),
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    std::fs::remove_dir_all(&cli.out).unwrap();

                    eprintln!(
                        "❌ {}\n{}",
                        "Could not fill project".bright_red(),
                        e.to_string().red(),
                    );
                }
            }

            match hook::run_hooks(config.hooks, &cli.out, data_entries) {
                Ok(_) => {
                    println!("🪝  Hooks executed successfully");
                }
                Err(e) => {
                    std::fs::remove_dir_all(&cli.out).unwrap();

                    eprintln!(
                        "❌ {} {}\n{}",
                        "Error running hook".bright_red(),
                        e.hook.name.bright_red().bold(),
                        e.error.to_string().red()
                    );

                    exit(1);
                }
            }
        }
    }
}
