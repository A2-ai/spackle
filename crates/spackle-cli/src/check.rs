use std::{path::Path, process::exit, time::Instant};

use colored::Colorize;
use spackle::{fs::StdFs, Severity};

use crate::diagnostic;

pub fn run(project_path: &Path) {
    println!("🔍 Validating project configuration\n");

    let start_time = Instant::now();
    let report = spackle::check_project(&StdFs::new(), project_path);

    if report.diagnostics.is_empty() {
        println!("  {}", "👌 No diagnostics — project looks good".dimmed());
        print_elapsed(start_time);
        return;
    }

    let error_count = report
        .diagnostics
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .count();
    let warning_count = report.diagnostics.len() - error_count;

    for d in &report.diagnostics {
        diagnostic::print(d);
    }

    let summary = format!(
        "Found {} error{}{}",
        error_count,
        if error_count == 1 { "" } else { "s" },
        if warning_count > 0 {
            format!(
                ", {} warning{}",
                warning_count,
                if warning_count == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        }
    );
    eprintln!("{}", summary.bright_red().bold());
    print_elapsed(start_time);

    if error_count > 0 {
        exit(1);
    }
}

fn print_elapsed(start_time: Instant) {
    println!(
        "  ✅ done {}",
        format!("in {:?}", start_time.elapsed()).dimmed()
    );
}
