use std::{collections::HashMap, fs, path::PathBuf, process::exit, time::Instant};

use colored::Colorize;
use rocket::{futures::StreamExt, tokio};
use spackle::core::{
    config::Config,
    copy,
    hook::{self, HookError, HookResult, HookResultKind, HookStreamResult},
    slot, template,
};
use tokio::pin;

use crate::{check, Cli};

pub fn run(
    slot: &Vec<String>,
    hook: &Vec<String>,
    project_dir: &PathBuf,
    out: &PathBuf,
    config: &Config,
    cli: &Cli,
) {
    // First, run spackle check
    check::run(project_dir, config);

    println!("");

    let slot_data = slot
        .iter()
        .filter_map(|data| match data.split_once('=') {
            Some((key, value)) => Some((key.to_string(), value.to_string())),
            None => {
                eprintln!(
                    "{} {}\n",
                    "❌",
                    "Invalid slot argument, must be in the form of key=value. Skipping."
                        .bright_red()
                );
                None
            }
        })
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<HashMap<String, String>>();

    match slot::validate_data(&slot_data, &config.slots) {
        Ok(()) => {}
        Err(e) => {
            eprintln!(
                "{}\n{}",
                "❌ Error with supplied data".bright_red(),
                e.to_string().red()
            );

            exit(1);
        }
    }

    let hook_data = hook
        .iter()
        .filter_map(|data| match data.split_once('=') {
            Some((key, value)) => Some((key.to_string(), value.to_string())),
            None => {
                eprintln!(
                    "{} {}\n",
                    "❌",
                    "Invalid hook argument, must be in the form of key=<true|false>. Skipping."
                        .bright_red()
                );
                None
            }
        })
        .filter_map(|(key, value)| match value.parse::<bool>() {
            Ok(v) => Some((key, v)),
            Err(_) => {
                eprintln!(
                    "{} {}\n",
                    "❌",
                    "Invalid hook argument, must be a boolean. Skipping.".bright_red()
                );
                None
            }
        })
        .collect::<HashMap<String, bool>>();

    // TODO validate hook data

    let start_time = Instant::now();

    let mut slot_data = slot_data.clone();
    slot_data.insert(
        "project_name".to_string(),
        project_dir.file_name().unwrap().to_string_lossy().into(),
    );

    // CR(devin): when looking at the below code, this likely should be pushed
    // into the spackle lib itself, there are too many implementation details
    // in the CLi that would also need to be replicated in any api/other client
    // when by the time you get to actually rendering the template
    // the fact this is touching like a util related module shows its
    // breaking the ideal implementation boundaries.

    // TODO: refactor the data_entries and context boundaries after considering
    // the api surface area
    match copy::copy(&project_dir, out, &config.ignore, &slot_data) {
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
            std::fs::remove_dir_all(out).unwrap();

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

    match template::fill(&project_dir, &PathBuf::from(out), &slot_data) {
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
                            format!("{}", e.kind).bright_yellow().dimmed(),
                        );
                    }
                }
            }
        }
        Err(e) => {
            std::fs::remove_dir_all(out).unwrap();

            eprintln!(
                "❌ {}\n{}",
                "Could not fill project".bright_red(),
                e.to_string().red(),
            );
        }
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
        let stream = match hook::run_hooks_stream(&config.hooks, out, &slot_data, &hook_data, None)
        {
            Ok(stream) => stream,
            Err(e) => {
                fs::remove_dir_all(out).unwrap();

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
                        fs::remove_dir_all(out).unwrap();

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
                            "{} {}\n",
                            "    ✅ done",
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
