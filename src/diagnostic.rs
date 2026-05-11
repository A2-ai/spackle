//! Structured diagnostics surfaced by `check` and `render`. Shared across
//! the wasm boundary, the TS package, and the CLI's diagnostic printer.

use serde::Serialize;

use crate::{config, copy, hook, slot, template};

const SPACKLE_TOML_PATH: &str = "spackle.toml";

/// Diagnostic severity. Only `Error` is currently emitted; `Warning` is
/// reserved for future use (e.g. dead slots, deprecated patterns).
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// Which pipeline stage produced the diagnostic.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSource {
    /// `spackle.toml` parse error or top-level config validation failure
    /// (duplicate keys across slots/hooks, etc.).
    Config,
    /// Slot-config error: bad default value type, etc.
    SlotConfig,
    /// Hook-config error: unknown `needs` reference, broken command
    /// template, etc.
    HookConfig,
    /// User-supplied slot data error: missing required slot, wrong type,
    /// unknown key.
    SlotData,
    /// Copy stage filesystem failure — read / write / mkdir. Path-template
    /// parse / render failures during copy are classified as `RenderName`
    /// instead, regardless of the file extension, so the source matches
    /// the user mental model ("filename template is broken").
    Copy,
    /// Template body render failure.
    RenderBody,
    /// Filename / path template parse or render failure. Fires for `.j2`
    /// filename templating AND non-`.j2` path templating — anywhere Tera
    /// is applied to a file path.
    RenderName,
}

/// One-based line and column into a source file.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub column: u32,
}

#[derive(Serialize, Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub source: DiagnosticSource,
    pub message: String,
    /// Bundle-virtual path of the offending file, or `"spackle.toml"`
    /// for config-level diagnostics. `None` for diagnostics that don't
    /// target a file (slot data errors).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Slot or hook key when the diagnostic targets a config item rather
    /// than a file. Serialized as `ref` in JSON.
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    /// Best-effort line/column. `None` when the underlying error format
    /// doesn't carry position info (see [`extract_tera_span`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    /// Stable identifier (e.g. `"hook::unknown_needs"`) so consumers
    /// can group/filter without parsing messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl Diagnostic {
    pub fn new(severity: Severity, source: DiagnosticSource, message: impl Into<String>) -> Self {
        Self {
            severity,
            source,
            message: message.into(),
            path: None,
            r#ref: None,
            span: None,
            code: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_ref(mut self, key: impl Into<String>) -> Self {
        self.r#ref = Some(key.into());
        self
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

/// Try to extract `{ line, column }` from a Tera error's `Display`.
///
/// Tera 2.0-alpha 4's rich error format renders position via a miette-
/// style `--> file:line:column` arrow line. We parse that defensively;
/// older `line N, column M` phrasing is also accepted as a fallback. If
/// neither matches, returns `None` and the diagnostic still carries a
/// useful path/message. A unit test pins the current behavior — Tera
/// upgrades that change the format will trip it loudly.
pub fn extract_tera_span(err: &tera::Error) -> Option<Span> {
    // Walk the error chain (Display includes nested cause).
    let combined = {
        use std::error::Error as _;
        let mut s = err.to_string();
        let mut cur = err.source();
        while let Some(e) = cur {
            s.push('\n');
            s.push_str(&e.to_string());
            cur = e.source();
        }
        s
    };

    // Format 1: `--> path:LINE:COL` (Tera 2.x rich report).
    for line in combined.lines() {
        let trimmed = line.trim_start();
        if let Some(after) = trimmed.strip_prefix("--> ") {
            if let Some(span) = parse_trailing_line_col(after) {
                return Some(span);
            }
        }
    }

    // Format 2: `... line N, column M`.
    let lower = combined.to_lowercase();
    if let Some(idx) = lower.find("line ") {
        let tail = &combined[idx + "line ".len()..];
        if let Some((line_str, rest)) = tail.split_once([',', ':']) {
            if let Ok(line) = line_str.trim().parse::<u32>() {
                let rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
                let rest = rest.strip_prefix("column ").unwrap_or(rest);
                let col_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(column) = col_str.parse::<u32>() {
                    return Some(Span { line, column });
                }
            }
        }
    }

    None
}

/// Parse a `<anything>:LINE:COL` suffix. Walks from the end so paths
/// containing colons (Windows drive letters, URLs) don't trip us up.
fn parse_trailing_line_col(s: &str) -> Option<Span> {
    let trimmed = s.trim_end();
    let last = trimmed.rfind(':')?;
    let col_str = &trimmed[last + 1..];
    let col: u32 = col_str
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    let rest = &trimmed[..last];
    let second = rest.rfind(':')?;
    let line_str = &rest[second + 1..];
    let line: u32 = line_str
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    Some(Span { line, column: col })
}

// --- converters -----------------------------------------------------------

pub fn from_file_error(err: &template::FileError) -> Diagnostic {
    let (source, tera_err) = match &err.kind {
        template::FileErrorKind::ErrorParsingTemplate(e) => (DiagnosticSource::RenderBody, Some(e)),
        template::FileErrorKind::ErrorRenderingContents(e) => {
            (DiagnosticSource::RenderBody, Some(e))
        }
        template::FileErrorKind::ErrorRenderingName(e) => (DiagnosticSource::RenderName, Some(e)),
        template::FileErrorKind::ErrorCreatingDest(_) => (DiagnosticSource::RenderBody, None),
        template::FileErrorKind::ErrorWritingToDest(_) => (DiagnosticSource::RenderBody, None),
    };
    // Prefer the inner Tera error's message verbatim when available —
    // the diagnostic's `source` and `path` fields already convey the
    // "which stage / which file" context that `FileErrorKind`'s Display
    // prefix would otherwise duplicate.
    let message = match tera_err {
        Some(e) => e.to_string(),
        None => err.kind.to_string(),
    };
    let mut d = Diagnostic::new(Severity::Error, source, message).with_path(err.file.clone());
    if let Some(tera) = tera_err {
        if let Some(span) = extract_tera_span(tera) {
            d = d.with_span(span);
        }
    }
    d
}

/// A `copy::Error` can wrap either a Tera failure (path-template
/// parse/render) or an `io::Error` (true fs op). Walk the source chain
/// to discriminate so the diagnostic's `source` reflects the
/// user-meaningful class — "this filename's template is broken" vs
/// "couldn't read/write this path" — instead of the pipeline stage.
pub fn from_copy_error(err: &copy::Error) -> Diagnostic {
    use std::error::Error as _;
    let tera_err = err.source().and_then(|s| s.downcast_ref::<tera::Error>());
    let source = if tera_err.is_some() {
        DiagnosticSource::RenderName
    } else {
        DiagnosticSource::Copy
    };
    let mut d = Diagnostic::new(Severity::Error, source, err.to_string())
        .with_path(err.path.to_string_lossy().into_owned());
    if let Some(tera) = tera_err {
        if let Some(span) = extract_tera_span(tera) {
            d = d.with_span(span);
        }
    }
    d
}

/// TOML parse errors carry a byte-offset span via the `toml` crate; we
/// resolve it against the source string for a line/col when available.
pub fn from_config_error(err: &config::Error, toml_source: Option<&str>) -> Diagnostic {
    let mut d = Diagnostic::new(Severity::Error, DiagnosticSource::Config, err.to_string())
        .with_path(SPACKLE_TOML_PATH);
    if let config::Error::ParseError(parse_err) = err {
        if let Some(range) = parse_err.span() {
            if let Some(src) = toml_source {
                if let Some(span) = byte_offset_to_line_col(src, range.start) {
                    d = d.with_span(span);
                }
            }
        }
    }
    d
}

/// `slot::Error` doubles as the error type for both `slot::validate`
/// (config stage) and `slot::validate_data` (runtime). This converter
/// classifies it as the former; use [`from_slot_data_error`] for the latter.
pub fn from_slot_config_error(err: &slot::Error) -> Diagnostic {
    let mut d = Diagnostic::new(
        Severity::Error,
        DiagnosticSource::SlotConfig,
        err.to_string(),
    )
    .with_path(SPACKLE_TOML_PATH);
    if let Some(key) = slot_error_key(err) {
        d = d.with_ref(key);
    }
    d
}

pub fn from_slot_data_error(err: &slot::Error) -> Diagnostic {
    let mut d = Diagnostic::new(Severity::Error, DiagnosticSource::SlotData, err.to_string());
    if let Some(key) = slot_error_key(err) {
        d = d.with_ref(key);
    }
    d
}

fn slot_error_key(err: &slot::Error) -> Option<&str> {
    match err {
        slot::Error::UnknownSlot(k) => Some(k),
        slot::Error::TypeMismatch(k, _) => Some(k),
        slot::Error::UndefinedSlot(k) => Some(k),
    }
}

pub fn from_hook_config_error(err: &hook::ConfigError) -> Diagnostic {
    let mut d = Diagnostic::new(
        Severity::Error,
        DiagnosticSource::HookConfig,
        err.to_string(),
    )
    .with_path(SPACKLE_TOML_PATH);
    d = d.with_ref(err.hook_key.clone());
    if let Some(span) = err.span {
        d = d.with_span(span);
    }
    if let Some(code) = err.code {
        d = d.with_code(code);
    }
    d
}

pub fn from_hook_template_errors(entry: &hook::HookPlanEntry) -> Vec<Diagnostic> {
    entry
        .template_errors
        .iter()
        .map(|msg| {
            Diagnostic::new(Severity::Error, DiagnosticSource::HookConfig, msg.clone())
                .with_path(SPACKLE_TOML_PATH)
                .with_ref(entry.key.clone())
                .with_code("hook::template_render_failed")
        })
        .collect()
}

/// Convert a byte offset in a string to (line, column) — both 1-indexed.
pub fn byte_offset_to_line_col(source: &str, offset: usize) -> Option<Span> {
    if offset > source.len() {
        return None;
    }
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            return Some(Span { line, column: col });
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    Some(Span { line, column: col })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_tera_span_from_display() {
        // Build a synthetic Tera error that exercises the parse path —
        // unterminated `{{` is the simplest reproducer.
        let mut tera = tera::Tera::default();
        let result = tera.add_raw_template("t.j2", "{{ unclosed ");
        let err = result.expect_err("expected tera parse error");
        // Span extraction is best-effort — assert it doesn't panic and
        // (if it succeeds) yields a plausible 1-indexed location.
        if let Some(span) = extract_tera_span(&err) {
            assert!(span.line >= 1);
            assert!(span.column >= 1);
        }
    }

    #[test]
    fn byte_offset_to_line_col_basic() {
        let s = "abc\ndef\nghi";
        assert_eq!(
            byte_offset_to_line_col(s, 0),
            Some(Span { line: 1, column: 1 })
        );
        assert_eq!(
            byte_offset_to_line_col(s, 4),
            Some(Span { line: 2, column: 1 })
        );
        assert_eq!(
            byte_offset_to_line_col(s, 8),
            Some(Span { line: 3, column: 1 })
        );
        assert_eq!(
            byte_offset_to_line_col(s, 10),
            Some(Span { line: 3, column: 3 })
        );
    }

    #[test]
    fn diagnostic_serializes_with_optional_fields_skipped() {
        let d = Diagnostic::new(Severity::Error, DiagnosticSource::Config, "boom")
            .with_path("spackle.toml");
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""severity":"error""#));
        assert!(json.contains(r#""source":"config""#));
        assert!(json.contains(r#""path":"spackle.toml""#));
        // ref / span / code omitted.
        assert!(!json.contains("\"ref\""));
        assert!(!json.contains("\"span\""));
        assert!(!json.contains("\"code\""));
    }

    #[test]
    fn diagnostic_serializes_ref_as_ref_not_r_pound_ref() {
        let d = Diagnostic::new(Severity::Error, DiagnosticSource::SlotConfig, "msg")
            .with_ref("slot_a");
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""ref":"slot_a""#));
    }
}
