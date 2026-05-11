use colored::Colorize;
use spackle::{Diagnostic, DiagnosticSource, Severity};

pub fn print(diagnostic: &Diagnostic) {
    let prefix = match diagnostic.severity {
        Severity::Error => "❌".to_string(),
        Severity::Warning => "⚠️".to_string(),
    };

    let location = format_location(diagnostic);
    let stage = source_label(diagnostic.source);

    let header = if location.is_empty() {
        format!("{} [{}]", prefix, stage)
    } else {
        format!("{} [{}] {}", prefix, stage, location)
    };

    let colored_header = match diagnostic.severity {
        Severity::Error => header.bright_red().to_string(),
        Severity::Warning => header.bright_yellow().to_string(),
    };

    let body = match diagnostic.severity {
        Severity::Error => diagnostic.message.red().to_string(),
        Severity::Warning => diagnostic.message.yellow().to_string(),
    };

    eprintln!("{}\n  {}\n", colored_header, body);
}

fn format_location(d: &Diagnostic) -> String {
    if let Some(path) = &d.path {
        match d.span {
            Some(span) => format!("{}:{}:{}", path, span.line, span.column),
            None => path.clone(),
        }
    } else if let Some(key) = &d.r#ref {
        format!("ref {}", key)
    } else {
        String::new()
    }
}

fn source_label(source: DiagnosticSource) -> &'static str {
    match source {
        DiagnosticSource::Config => "config",
        DiagnosticSource::SlotConfig => "slot",
        DiagnosticSource::HookConfig => "hook",
        DiagnosticSource::SlotData => "slot data",
        DiagnosticSource::Copy => "copy",
        DiagnosticSource::RenderBody => "render",
        DiagnosticSource::RenderName => "render filename",
    }
}
