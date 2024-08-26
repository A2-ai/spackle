use crate::{check, Cli};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Input};
use fronma::parser::parse_with_engine;
use rocket::{futures::StreamExt, tokio};
use spackle::{
    core::{
        config::{self, Config},
        copy,
        hook::{self, HookError, HookResult, HookResultKind, HookStreamResult},
        slot::{self, Slot, SlotType},
        template,
    },
    get_project_name,
};
use std::{collections::HashMap, fs, path::PathBuf, process::exit, time::Instant};
use tera::{Context, Tera};
use tokio::pin;

fn collect_slot_data(slot: &Vec<String>, slots: Vec<Slot>) -> HashMap<String, String> {
    let mut slot_data = slot
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

    // at this point we've collected all the flags, so we should identify
    // if any additional slots are needed and if we're in a tty context prompt
    // for more slot info before validating
    if atty::is(atty::Stream::Stdout) {
        let missing_slots: Vec<Slot> = slots
            .into_iter()
            .filter(|slot| !slot_data.contains_key(&slot.key))
            .collect();

        missing_slots.iter().for_each(|slot| {
            match &slot.r#type {
                SlotType::String => {
                    let input = Input::with_theme(&ColorfulTheme::default())
                        .with_prompt(&slot.get_name())
                        .interact_text()
                        .unwrap();

                    slot_data.insert(slot.key.clone(), input);
                }
                SlotType::Boolean => {
                    let input = Input::with_theme(&ColorfulTheme::default())
                        .with_prompt(&slot.get_name())
                        .validate_with(|input: &String| -> Result<(), &str> {
                            // ensure input is a boolean
                            if input.parse::<bool>().is_err() {
                                return Err("Input must be a boolean".into());
                            }
                            Ok(())
                        })
                        .interact()
                        .unwrap();

                    slot_data.insert(slot.key.clone(), input);
                }
                SlotType::Number => {
                    let input = Input::with_theme(&ColorfulTheme::default())
                        .with_prompt(&slot.get_name())
                        .validate_with(|input: &String| -> Result<(), &str> {
                            if input.parse::<i32>().is_err() {
                                return Err("Input must be a number".into());
                            }
                            Ok(())
                        })
                        .interact_text()
                        .unwrap();

                    slot_data.insert(slot.key.clone(), input);
                }
            }
        });
    }

    println!();

    slot_data
}

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

    let mut slot_data = collect_slot_data(slot, config.slots.clone());

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

    slot_data.insert("_project_name".to_string(), get_project_name(project_dir));

    if cli.project_path.is_dir() {
        run_multi(&slot_data, hook, project_dir, out, config, cli);
    } else {
        run_single(&slot_data, &cli)
    }
}

pub fn run_multi(
    slot_data: &HashMap<String, String>,
    hook: &Vec<String>,
    project_dir: &PathBuf,
    out: &PathBuf,
    config: &Config,
    cli: &Cli,
) {
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
                if r.copied_count == 1 { "file" } else { "files" },
                format!("in {:?}", start_time.elapsed()).dimmed()
            );

            if r.skipped_count > 0 {
                println!(
                    "{}",
                    format!(
                        "{} {} {}",
                        "  Ignored",
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
            std::fs::remove_dir_all(out).unwrap();

            eprintln!(
                "❌ {}\n{}",
                "Could not fill project".bright_red(),
                e.to_string().red(),
            );
        }
    }

    if config.hooks.is_empty() {
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

pub fn run_single(slot_data: &HashMap<String, String>, cli: &Cli) {
    let start_time = Instant::now();

    let file_contents = match fs::read_to_string(&cli.project_path) {
        Ok(o) => o,
        Err(e) => {
            eprintln!(
                "❌ {}\n{}",
                "Error reading project file".bright_red(),
                e.to_string().red()
            );
            exit(1);
        }
    };

    let body = match parse_with_engine::<config::Config, fronma::engines::Toml>(&file_contents) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("❌ {}\n{:#?}", "Error parsing project file".bright_red(), e);
            exit(1);
        }
    }
    .body;

    let context = match Context::from_serialize(slot_data) {
        Ok(context) => context,
        Err(e) => {
            eprintln!(
                "❌ {}\n{}",
                "Error parsing context".bright_red(),
                e.to_string().red()
            );
            exit(1);
        }
    };

    let result = match Tera::one_off(body, &context, false) {
        Ok(result) => result,
        Err(e) => {
            eprintln!(
                "❌ {}\n{}",
                "Error rendering template".bright_red(),
                e.to_string().red()
            );
            exit(1);
        }
    };

    let output_path = cli
        .out_dir
        .join(get_project_name(&cli.project_path).as_str());

    // TODO do we want to output to out_dir to make consistent with full project render?
    // match fs::write(&output_path, result.clone()) {
    //     Ok(_) => {}
    //     Err(e) => {
    //         eprintln!(
    //             "❌ {}\n{}",
    //             "Error writing output file".bright_red(),
    //             e.to_string().red()
    //         );
    //         exit(1);
    //     }
    // }

    println!(
        "⛽ Rendered file {}\n  {}",
        format!("in {:?}", start_time.elapsed()).dimmed(),
        output_path.to_string_lossy().bold()
    );

    // if cli.verbose {
    println!("\n{}\n{}", "contents".dimmed(), result);
    // }
}
