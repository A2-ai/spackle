use crate::{check, util::file_path_completer::FilePathCompleter, Cli};
use anyhow::{Context, Result};
use colored::Colorize;
use fronma::parser::parse_with_engine;
use inquire::{Confirm, CustomType, Text};
use rocket::{futures::StreamExt, tokio};
use spackle::{
    config::{self},
    hook::{self, Hook, HookError, HookResult, HookResultKind, HookStreamResult},
    slot::{self, Slot, SlotType},
    Project,
};
use std::{collections::HashMap, fs, path::PathBuf, process::exit, time::Instant};
use tera::Tera;
use tokio::pin;

fn parse_flag_data(flag_data: &Vec<String>) -> HashMap<String, String> {
    flag_data
        .iter()
        .filter_map(|e| match e.split_once('=') {
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
        .collect()
}

fn collect_data(
    flag_data: &Vec<String>,
    slots: &Vec<Slot>,
    hooks: &Vec<Hook>,
) -> Result<HashMap<String, String>> {
    let mut collected: HashMap<String, String> = HashMap::new();

    for (key, value) in parse_flag_data(flag_data) {
        collected.insert(key, value);
    }

    // at this point we've collected all the flags, so we should identify
    // if any additional slots are needed and if we're in a tty context prompt
    // for more slot info before validating
    if atty::is(atty::Stream::Stdout) {
        println!("📮 Collecting data\n");

        let missing_slots: Vec<&Slot> = slots
            .iter()
            .filter(|slot| !collected.contains_key(&slot.key))
            .collect();

        for slot in missing_slots {
            match &slot.r#type {
                SlotType::String => {
                    let slot_name = slot.get_name();
                    let mut input = Text::new(&slot_name);

                    if let Some(description) = &slot.description {
                        input = input.with_help_message(description);
                    }

                    if let Some(default) = &slot.default {
                        // We can unwrap here because we've done prior validation
                        input = input.with_default(default);
                    }

                    let value = input
                        .prompt()
                        .with_context(|| format!("Error getting input for slot: {}", slot.key))?;

                    collected.insert(slot.key.clone(), value.to_string());
                }
                SlotType::Boolean => {
                    let slot_name = slot.get_name();
                    let mut input = Confirm::new(&slot_name);

                    if let Some(description) = &slot.description {
                        input = input.with_help_message(description);
                    }

                    if let Some(default) = &slot.default {
                        // We can unwrap here because we've done prior validation
                        input = input.with_default(default.parse::<bool>().unwrap());
                    }

                    let value = input
                        .prompt()
                        .with_context(|| format!("Error getting input for slot: {}", slot.key))?;

                    collected.insert(slot.key.clone(), value.to_string());
                }
                SlotType::Number => {
                    let slot_name = slot.get_name();
                    let mut input = CustomType::<f64>::new(&slot_name)
                        .with_error_message("Please type a valid number");

                    if let Some(description) = &slot.description {
                        input = input.with_help_message(description);
                    }

                    if let Some(default) = &slot.default {
                        // We can unwrap here because we've done prior validation
                        input = input.with_default(default.parse::<f64>().unwrap());
                    }

                    let value = input
                        .prompt()
                        .with_context(|| format!("Error getting input for slot: {}", slot.key))?;

                    collected.insert(slot.key.clone(), value.to_string());
                }
            }
        }
    }

    for hook in hooks {
        let prompt = format!("Run {}?", hook.name.clone().unwrap_or(hook.key.clone()));
        let mut input = Confirm::new(prompt.as_str());

        if let Some(description) = &hook.description {
            input = input.with_help_message(description);
        }

        if let Some(default) = hook.default {
            // We can unwrap here because we've done prior validation
            input = input.with_default(default)
        }

        let value = input
            .prompt()
            .with_context(|| format!("Error getting input for hook: {}", hook.key))?;

        collected.insert(hook.key.clone(), value.to_string());
    }

    Ok(collected)
}

pub fn run(
    flag_data: &Vec<String>,
    overwrite: &bool,
    out_path: &Option<PathBuf>,
    project: &Project,
    cli: &Cli,
) {
    // First, run spackle check
    check::run(project);

    println!("");

    let collected_data = match collect_data(flag_data, &project.config.slots, &project.config.hooks)
    {
        Ok(slot_data) => slot_data,
        Err(e) => {
            eprintln!("❌ {}", format!("{:?}", e).red());
            exit(1);
        }
    };

    let slot_data: HashMap<String, String> = collected_data
        .iter()
        .filter(|(key, _)| project.config.slots.iter().any(|slot| slot.key == **key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

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

    let hook_data: HashMap<String, String> = collected_data
        .iter()
        .filter(|(key, _)| project.config.hooks.iter().any(|hook| hook.key == **key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Err(e) = hook::validate_data(&hook_data, &project.config.hooks) {
        eprintln!(
            "{}\n{}",
            "❌ Error with supplied hook data".bright_red(),
            e.to_string().red()
        );

        exit(1);
    }

    // Check if any data entries don't align with slots or hooks
    let unknown_data: Vec<&String> = collected_data
        .iter()
        .filter(|(key, _)| !slot_data.contains_key(*key) && !hook_data.contains_key(*key))
        .map(|(key, _)| key)
        .collect();

    if !unknown_data.is_empty() {
        eprintln!(
            "{}\n{}\n{}\n",
            "⚠️ Unrecognized data provided".bright_yellow(),
            "Please ensure all data passed via the --data (-d) flag corresponds to a slot or hook. Unrecognized:".yellow(),
            unknown_data
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<String>>()
                .join(", ")
                .yellow()
                .dimmed(),
        );
    }

    let out_path = match &out_path {
        Some(path) => path,
        // Cannot use CustomType here because PathBuf does not implement ToString
        None => {
            println!("📮 Collecting output path\n");

            let path = &Text::new("Enter the output path")
                .with_help_message("The path to output the filled project")
                .with_autocomplete(FilePathCompleter::default())
                .prompt();

            println!();

            match path {
                Ok(p) => &PathBuf::from(p),
                Err(e) => {
                    eprintln!("❌ {}", e.to_string().red());
                    exit(1);
                }
            }
        }
    };

    println!("");

    // Ensure the output path doesn't exist
    if *overwrite {
        println!(
            "{}\n",
            format!("⚠️ Overwriting existing output path").yellow()
        );
    } else if out_path.exists() {
        eprintln!(
            "{}\n{}",
            "❌ Path already exists".bright_red(),
            "Please remove the path before running spackle again".red()
        );

        exit(2);
    }

    // Create all parent directories
    if let Some(parent) = out_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            eprintln!("❌ {}", e.to_string().red());
            exit(1);
        }
    }

    if cli.project_path.is_dir() {
        run_multi(&collected_data, out_path, cli, project);
    } else {
        run_single(&slot_data, out_path, cli);
    }
}

pub fn run_multi(data: &HashMap<String, String>, out_dir: &PathBuf, cli: &Cli, project: &Project) {
    let start_time = Instant::now();

    println!("🖨️  Creating project files\n");
    println!(
        "{}",
        format!("  📁 {}", out_dir.to_string_lossy().bold()).dimmed()
    );

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
                                eprintln!(
                                    "\n    {}\n{}",
                                    "stdout".bold().dimmed(),
                                    String::from_utf8_lossy(&stdout)
                                );
                                eprintln!(
                                    "    {}\n{}",
                                    "stderr".bold().dimmed(),
                                    String::from_utf8_lossy(&stderr)
                                );
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
                            println!(
                                "    {}\n{}",
                                "stdout".bold().dimmed(),
                                String::from_utf8_lossy(&stdout)
                            );
                            println!(
                                "    {}\n{}",
                                "stderr".bold().dimmed(),
                                String::from_utf8_lossy(&stderr)
                            );
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

pub fn run_single(slot_data: &HashMap<String, String>, out_path: &PathBuf, cli: &Cli) {
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

    let context = match tera::Context::from_serialize(slot_data) {
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

    match fs::write(&out_path, result.clone()) {
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "❌ {}\n{}",
                "Error writing output file".bright_red(),
                e.to_string().red()
            );
            exit(1);
        }
    }

    println!(
        "⛽ Rendered file {}\n  {}",
        format!("in {:?}", start_time.elapsed()).dimmed(),
        out_path.to_string_lossy().bold()
    );

    if cli.verbose {
        println!("\n{}\n{}", "contents".dimmed(), result);
    }
}
