use colored::Colorize;
use rocket::{futures::StreamExt, tokio};
use spackle::{
    hook::{self, HookError, HookResult, HookResultKind, HookStreamResult},
    slot, Project,
};

use std::{collections::HashMap, fs, path::PathBuf, process::exit, time::Instant};
use tokio::pin;

use crate::{check, Cli};

pub fn run(data: &Vec<String>, overwrite: bool, out_dir: &PathBuf, project: &Project, cli: &Cli) {
    // First, run spackle check
    check::run(project);

    println!();

    let data = data
        .iter()
        .filter_map(|data| match data.split_once('=') {
            Some((key, value)) => Some((key.to_string(), value.to_string())),
            None => {
                eprintln!(
                    "❌ {}\n",
                    "Invalid data argument, must be in the form of key=value. Skipping."
                        .bright_red()
                );
                None
            }
        })
        .collect::<HashMap<String, String>>();

    let slot_data = data
        .clone()
        .into_iter()
        .filter(|(key, _)| project.config.slots.iter().any(|s| s.key == key.as_str()))
        .collect::<HashMap<String, String>>();

    if let Err(e) = slot::validate_data(&slot_data, &project.config.slots) {
        eprintln!(
            "{}\n{}",
            "❌ Error with supplied slot data".bright_red(),
            e.to_string().red()
        );

        if let slot::Error::UndefinedSlot(key) = e {
            println!(
                "{}",
                format!(
                    "\nℹ Define a value for {} using the --data (-d) flag\ne.g. --data {}=<value>",
                    key.to_string().bold(),
                    key
                )
                .yellow()
            );
        }

        exit(1);
    }

    let hook_data = data
        .clone()
        .into_iter()
        .filter(|(key, _)| project.config.hooks.iter().any(|h| h.key == key.as_str()))
        .collect::<HashMap<String, String>>();

    if let Err(e) = hook::validate_data(&hook_data, &project.config.hooks) {
        eprintln!(
            "{}\n{}",
            "❌ Error with supplied hook data".bright_red(),
            e.to_string().red()
        );

        exit(1);
    }

    let start_time = Instant::now();

    let mut data = data.clone();
    data.insert("project_name".to_string(), project.get_name());

    println!("🖨️  Creating project files\n");

    println!(
        "{}",
        format!("  📁 {}\n", out_dir.to_string_lossy().bold()).dimmed()
    );

    // Ensure the output directory is not the same as the project directory
    if out_dir == &cli.project_dir {
        eprintln!(
            "{}\n{}",
            "❌ Output directory cannot be the same as project directory".bright_red(),
            "Please choose a different output directory.".red()
        );
        exit(2);
    }

    if overwrite {
        println!(
            "{}\n",
            format!("  ⚠️ Overwriting existing directory").yellow()
        );
    } else if out_dir.exists() {
        eprintln!(
            "{}\n{}",
            "  ❌ Directory already exists".bright_red(),
            "  Please remove the directory before running spackle again".red()
        );

        exit(1);
    }

    match project.copy_files(out_dir, &data) {
        Ok(r) => {
            println!(
                "  Copied {} {} {}",
                r.copied_count,
                if r.copied_count == 1 { "file" } else { "files" },
                format!("in {:?}", start_time.elapsed()).dimmed()
            );

            if r.skipped_count > 0 {
                println!(
                    "{}",
                    format!(
                        "{} {} {}",
                        "    Ignored",
                        r.skipped_count,
                        if r.skipped_count == 1 {
                            "entry"
                        } else {
                            "entries"
                        }
                    )
                    .to_string()
                    .dimmed()
                );
            }

            println!();
        }
        Err(e) => {
            let _ = fs::remove_dir_all(out_dir);

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

    match project.render_templates(&PathBuf::from(out_dir), &data) {
        Ok(r) => {
            println!(
                "  Rendered {} {} {} {}\n",
                r.len(),
                if r.len() == 1 { "file" } else { "files" },
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
                            format!("{}", e.kind).bright_yellow().dimmed(),
                        );
                    }
                }
            }
        }
        Err(e) => {
            let _ = fs::remove_dir_all(out_dir);

            eprintln!(
                "❌ {}\n{}",
                "Could not fill project".bright_red(),
                e.to_string().red(),
            );
        }
    }

    // print done
    println!(
        "  ✅ done {}\n",
        format!("{:?}", start_time.elapsed()).dimmed()
    );

    if project.config.hooks.is_empty() {
        println!("🪝  No hooks to run");
        return;
    }

    println!("🪝  Running hooks...\n");

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("{}", e.to_string().red());
            exit(1);
        }
    };

    runtime.block_on(async {
        let stream = match project.run_hooks_stream(out_dir, &data, None) {
            Ok(stream) => stream,
            Err(e) => {
                let _ = fs::remove_dir_all(out_dir);

                eprintln!(
                    "  ❌ {}\n  {}",
                    "Error evaluating hooks".bright_red(),
                    e.to_string().red()
                );

                exit(1);
            }
        };
        pin!(stream);

        let mut start_time = Instant::now();

        while let Some(result) = stream.next().await {
            match result {
                HookStreamResult::HookStarted(hook) => {
                    println!("  🚀 {}", hook);
                }
                HookStreamResult::HookDone(r) => match r {
                    HookResult {
                        hook,
                        kind: HookResultKind::Failed(error),
                        ..
                    } => {
                        eprintln!(
                            "    ❌ {}\n    {}",
                            format!("Hook {} failed", hook.key.bold()).bright_red(),
                            error.to_string().red()
                        );

                        if cli.verbose {
                            if let HookError::CommandExited { stdout, stderr, .. } = error {
                                eprintln!("\n    {}\n{}", "stdout".bold().dimmed(), stdout);
                                eprintln!("    {}\n{}", "stderr".bold().dimmed(), stderr);
                            }
                        }

                        exit(1);
                    }
                    HookResult {
                        kind: HookResultKind::Completed { stdout, stderr },
                        ..
                    } => {
                        println!(
                            "    ✅ done {}\n",
                            format!("in {:?}", start_time.elapsed()).dimmed()
                        );

                        if cli.verbose {
                            println!("    {}\n{}", "stdout".bold().dimmed(), stdout);
                            println!("    {}\n{}", "stderr".bold().dimmed(), stderr);
                        }
                    }
                    HookResult {
                        kind: HookResultKind::Skipped(reason),
                        ..
                    } => {
                        println!("    ⏩︎ skipping {}\n", reason.to_string().dimmed());
                    }
                },
            };

            start_time = Instant::now();
        }
    });
}
