use super::slot::Slot;
#[cfg(not(target_arch = "wasm32"))]
use async_process::Stdio;
#[cfg(not(target_arch = "wasm32"))]
use async_stream::stream;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Display};
#[cfg(not(target_arch = "wasm32"))]
use std::{io, path::Path, process};
use tera::{Context, Tera};
#[cfg(not(target_arch = "wasm32"))]
use thiserror::Error;
#[cfg(not(target_arch = "wasm32"))]
use tokio::pin;
#[cfg(not(target_arch = "wasm32"))]
use tokio_stream::{Stream, StreamExt};
#[cfg(not(target_arch = "wasm32"))]
use users::User;

use crate::needs::{is_satisfied, Needy};

/// A hook's command, as written in `spackle.toml`. Two forms:
///
/// - `command = "echo {{ name }} && echo hi"` — a single string. Templated
///   as a whole, then run via `bash -c`. Slot values substitute as **raw
///   shell text**; the author owns quoting (`'{{ name }}'` for literal).
/// - `command = ["echo", "{{ name }}", "&&", "echo", "hi"]` — an argv
///   array. Each element is templated, then POSIX-quoted (bare shell
///   operators pass through), then joined and run via `bash -c`. Slot
///   values are **literal arguments** and can't act as shell syntax.
///
/// An array of the shape `["bash"|"sh", "-c", body]` is a pass-through:
/// `body` is templated and re-wrapped, equivalent to the string form.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum HookCommand {
    String(String),
    Array(Vec<String>),
}

impl HookCommand {
    /// Template-bearing parts, for parse-only validation.
    fn template_parts(&self) -> Vec<&str> {
        match self {
            HookCommand::String(s) => vec![s.as_str()],
            HookCommand::Array(args) => args.iter().map(|s| s.as_str()).collect(),
        }
    }

    /// Un-templated shell body, for the static denylist scan and `Display`.
    /// Mirrors [`render_command`]'s quoting (just without templating) so the
    /// static scan matches what would actually execute: array elements are
    /// POSIX-quoted (operators pass through), which keeps a literal argument
    /// like `["echo", "a; rm -rf /"]` from looking like a chained command.
    fn raw_body(&self) -> String {
        match self {
            HookCommand::String(s) => s.clone(),
            HookCommand::Array(args) if is_shell_wrapper(args) => args[2].clone(),
            HookCommand::Array(args) => args
                .iter()
                .map(|a| {
                    if SHELL_OPERATORS.contains(&a.as_str()) {
                        a.clone()
                    } else {
                        posix_quote(a)
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Argv to surface in a plan entry for a hook that will NOT run. Runnable
    /// hooks carry their rendered `["bash", "-c", body]`; this is only for
    /// skipped/errored entries where no body was rendered.
    pub fn display_argv(&self) -> Vec<String> {
        match self {
            HookCommand::String(s) => vec!["bash".to_string(), "-c".to_string(), s.clone()],
            HookCommand::Array(args) => args.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Hook {
    pub key: String,
    pub command: HookCommand,
    pub r#if: Option<String>,
    #[serde(default)]
    pub needs: Vec<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub default: Option<bool>,
}

impl Display for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}\n{}",
            self.key.bold(),
            if let Some(default) = &self.default {
                format!(
                    "default {}",
                    if *default { "on".green() } else { "off".red() }
                )
            } else {
                "".to_string()
            }
            .dimmed(),
            self.command.raw_body().dimmed()
        )
    }
}

impl Default for Hook {
    fn default() -> Self {
        Hook {
            key: "".to_string(),
            command: HookCommand::Array(vec![]),
            r#if: None,
            needs: vec![],
            name: None,
            description: None,
            default: None,
        }
    }
}

impl Needy for Hook {
    fn key(&self) -> String {
        self.key.clone()
    }

    fn is_enabled(&self, data: &HashMap<String, String>) -> bool {
        if data.contains_key(&self.key) {
            return data[&self.key] == "true";
        }

        self.default.unwrap_or(true)
    }

    fn is_satisfied(&self, items: &Vec<&dyn Needy>, data: &HashMap<String, String>) -> bool {
        is_satisfied(&self.needs, items, data)
    }
}

impl Hook {
    /// Evaluate the `if = "..."` expression against `context`. Returns
    /// `Ok(true)` when the hook has no `if` field. `pub` so alternative
    /// planners (e.g. `spackle-wasm`'s native-parity planner) can use it.
    pub fn evaluate_conditional(
        &self,
        context: &HashMap<String, String>,
    ) -> Result<bool, ConditionalError> {
        let conditional = match &self.r#if {
            Some(conditional) => conditional,
            None => return Ok(true),
        };

        let context = Context::from_serialize(context).map_err(ConditionalError::InvalidContext)?;

        let condition_str = Tera::one_off(conditional, &context, false)
            .map_err(ConditionalError::InvalidTemplate)?;

        let condition = condition_str
            .trim()
            .parse::<bool>()
            .map_err(|e| ConditionalError::NotBoolean(e.to_string()))?;

        Ok(condition)
    }
}

const SHELL_OPERATORS: &[&str] = &["&&", "||", "|", ";"];

#[derive(Debug, thiserror::Error)]
pub enum HookCommandError {
    #[error("command matches a blocked dangerous pattern ({pattern}); refusing to run: {matched}")]
    DangerousPattern {
        pattern: &'static str,
        matched: String,
    },
}

/// POSIX single-quote a string so it survives `bash -c` as one literal
/// argument. Safe characters pass through unquoted; everything else is
/// wrapped in `'...'` with embedded single quotes escaped as `'\''`.
fn posix_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '-' | '/' | '.' | ':' | '=' | '@' | ',' | '+')
    }) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn is_shell_wrapper(args: &[String]) -> bool {
    args.len() >= 3 && (args[0] == "bash" || args[0] == "sh") && args[1] == "-c"
}

/// Render a [`HookCommand`] into the full argv to spawn — always
/// `["bash", "-c", <body>, <positional>...]`. The executable shell text is
/// always at index 2, which is what [`dangerous_pattern_check`] must scan.
///
/// - String form: template the whole string (raw substitution).
/// - `["bash"|"sh", "-c", body, pos...]`: template `body` and any positional
///   args (pass-through). The `-c` body is taken verbatim so the denylist
///   sees the real command, not a quoted-inside-out version of it.
/// - Array form: template each element, then POSIX-quote (bare shell
///   operators pass through), then join into a single body. This
///   template-then-quote order makes slot values literal arguments — they
///   can't act as shell syntax.
pub fn render_command(
    command: &HookCommand,
    context: &Context,
) -> Result<Vec<String>, tera::Error> {
    let mut argv = vec!["bash".to_string(), "-c".to_string()];
    match command {
        HookCommand::String(s) => {
            argv.push(Tera::one_off(s, context, false)?);
        }
        HookCommand::Array(args) if is_shell_wrapper(args) => {
            // args[2] is the `-c` body; args[3..] are bash positional params
            // ($0, $1, ...). Render each and forward them as real argv so the
            // body executes as the author wrote it.
            for arg in &args[2..] {
                argv.push(Tera::one_off(arg, context, false)?);
            }
        }
        HookCommand::Array(args) => {
            let mut parts = Vec::with_capacity(args.len());
            for arg in args {
                if SHELL_OPERATORS.contains(&arg.as_str()) {
                    parts.push(arg.clone());
                } else {
                    parts.push(posix_quote(&Tera::one_off(arg, context, false)?));
                }
            }
            argv.push(parts.join(" "));
        }
    }
    Ok(argv)
}

/// Refuse a small denylist of catastrophic shell patterns in a rendered
/// command body. This is the only safety net: hooks run as the target user,
/// so the blast radius is what that user could already type at their shell —
/// but a templated `rm -rf /` or fork bomb is almost never intended, so we
/// block the unambiguous cases.
pub fn dangerous_pattern_check(rendered: &str) -> Result<(), HookCommandError> {
    let collapsed: String = rendered.chars().filter(|c| !c.is_whitespace()).collect();
    if collapsed.contains(":(){:|:&};:") {
        return Err(HookCommandError::DangerousPattern {
            pattern: "fork bomb",
            matched: rendered.trim().to_string(),
        });
    }
    if let Some(matched) = detect_rm_rf_root(rendered) {
        return Err(HookCommandError::DangerousPattern {
            pattern: "recursive force-remove of a root/system path",
            matched,
        });
    }
    Ok(())
}

/// True for targets that a `rm -rf` should never touch: `/`, `/*`, or a
/// top-level system directory (with optional trailing `/` or `/*`).
fn is_dangerous_rm_target(target: &str) -> bool {
    let core = target.trim_end_matches("/*").trim_end_matches('/');
    if core.is_empty() {
        return true; // "/", "//", "/*"
    }
    const TOP_LEVEL: &[&str] = &[
        "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/proc", "/root", "/sbin",
        "/sys", "/usr", "/var",
    ];
    TOP_LEVEL.contains(&core)
}

/// Is this `rm` segment recursive AND forced AND aimed at a root/system path?
fn rm_segment_is_dangerous(segment: &[&str]) -> bool {
    let (mut recursive, mut force, mut root_target) = (false, false, false);
    for arg in segment {
        if let Some(long) = arg.strip_prefix("--") {
            match long {
                "recursive" => recursive = true,
                "force" => force = true,
                _ => {}
            }
        } else if let Some(short) = arg.strip_prefix('-') {
            if short.contains(['r', 'R']) {
                recursive = true;
            }
            if short.contains('f') {
                force = true;
            }
        } else if is_dangerous_rm_target(arg) {
            root_target = true;
        }
    }
    recursive && force && root_target
}

/// Split a shell body into command segments at *unquoted* command separators
/// (`;`, `&`, `&&`, `|`, `||`, newline). Single and double quotes — and
/// backslash escapes outside single quotes — suppress separator recognition,
/// so a separator inside a quoted argument stays part of one segment. This is
/// a best-effort scanner for the denylist, not a full shell parser, but it
/// matches separators whether or not they are whitespace-padded
/// (`a;rm`, `a && rm`, a trailing newline, …).
fn split_command_segments(body: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = body.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        if in_single {
            current.push(c);
            if c == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            current.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    current.push(n);
                }
            } else if c == '"' {
                in_double = false;
            }
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                current.push(c);
            }
            '"' => {
                in_double = true;
                current.push(c);
            }
            '\\' => {
                current.push(c);
                if let Some(n) = chars.next() {
                    current.push(n);
                }
            }
            ';' | '\n' | '&' | '|' => {
                // Consume a doubled operator (`&&` / `||`) as one separator.
                if (c == '&' || c == '|') && chars.peek() == Some(&c) {
                    chars.next();
                }
                segments.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    segments.push(current);
    segments
}

/// Scan a rendered command body for a recursive, forced `rm` of a
/// root/system path. Splits into command segments so chained commands are
/// considered independently, then for each segment checks whether the leading
/// command — after transparent `sudo`/`doas` — is such an `rm`. Quoted
/// separators don't split, so a literal argument like `echo 'a; rm -rf /'` is
/// correctly treated as data, while `echo a; rm -rf /` is caught.
fn detect_rm_rf_root(rendered: &str) -> Option<String> {
    for segment in split_command_segments(rendered) {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let mut idx = 0;
        while matches!(tokens.get(idx), Some(&"sudo") | Some(&"doas")) {
            idx += 1;
        }
        if tokens.get(idx) == Some(&"rm") {
            let args = &tokens[idx + 1..];
            if rm_segment_is_dangerous(args) {
                return Some(format!("rm {}", args.join(" ")));
            }
        }
    }
    None
}

#[derive(Serialize, Debug)]
pub enum ConditionalError {
    InvalidContext(#[serde(skip)] tera::Error),
    InvalidTemplate(#[serde(skip)] tera::Error),
    NotBoolean(String),
}

impl Display for ConditionalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConditionalError::InvalidContext(e) => write!(f, "invalid context\n{}", e),
            ConditionalError::InvalidTemplate(e) => write!(f, "invalid template\n{}", e),
            ConditionalError::NotBoolean(e) => write!(f, "not a boolean\n{}", e),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Debug)]
pub struct HookResult {
    pub hook: Hook,
    pub kind: HookResultKind,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Debug)]
pub enum HookResultKind {
    Skipped(SkipReason),
    Completed { stdout: Vec<u8>, stderr: Vec<u8> },
    Failed(HookError),
}

#[cfg(not(target_arch = "wasm32"))]
impl Display for HookResultKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookResultKind::Skipped(reason) => write!(f, "skipped: {}", reason),
            HookResultKind::Completed { .. } => {
                write!(f, "completed")
            }
            HookResultKind::Failed(e) => write!(f, "failed: {}", e),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Debug)]
#[serde(tag = "type")]
pub enum HookError {
    ConditionalFailed(ConditionalError),
    CommandLaunchFailed(#[serde(skip)] io::Error),
    CommandExited {
        exit_code: i32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookError::ConditionalFailed(e) => write!(f, "conditional failed: {}", e),
            HookError::CommandLaunchFailed(e) => write!(f, "command launch failed: {}", e),
            HookError::CommandExited { exit_code, .. } => {
                write!(f, "command exited with code {}", exit_code)
            }
        }
    }
}

#[derive(Serialize, Debug)]
pub enum SkipReason {
    UserDisabled,
    FalseConditional,
}

impl Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::UserDisabled => write!(f, "user disabled"),
            SkipReason::FalseConditional => write!(f, "false conditional"),
        }
    }
}

/// Entry in a hook execution plan: describes what would happen if hooks
/// were run, without actually spawning any processes.
#[derive(Serialize, Debug)]
pub struct HookPlanEntry {
    pub key: String,
    pub command: Vec<String>,
    pub should_run: bool,
    pub skip_reason: Option<String>,
    /// Template errors encountered while rendering command args. Non-empty
    /// means the command may not be correct. Native execution treats these
    /// as hard errors; the plan surfaces them for the caller to decide.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub template_errors: Vec<String>,
}

/// Evaluate which hooks would run and in what order, without executing them.
/// Pure computation: resolves needs, evaluates conditionals, templates
/// command args.
///
/// To match native execution semantics, the evaluator injects
/// `hook_ran_<key> = "true"` into the context for each prior hook that
/// was planned to run. This lets conditionals like
/// `if = "{{ hook_ran_create_repo }}"` evaluate correctly under the
/// assumption that all prior hooks succeed (best-case plan).
pub fn evaluate_hook_plan(
    hooks: &[Hook],
    slots: &[Slot],
    data: &HashMap<String, String>,
) -> Vec<HookPlanEntry> {
    let items: Vec<&dyn Needy> = {
        let mut items = slots
            .iter()
            .map(|s| s as &dyn Needy)
            .collect::<Vec<&dyn Needy>>();
        items.extend(hooks.iter().map(|h| h as &dyn Needy));
        items
    };

    // Running context that accumulates hook_ran_* state as we plan.
    let mut running_data = data.clone();
    // Pre-populate hook_ran_* = "false" for all hooks so the variable
    // always exists in the context (prevents "undefined" errors).
    for hook in hooks {
        running_data
            .entry(format!("hook_ran_{}", hook.key))
            .or_insert_with(|| "false".to_string());
    }

    let mut results = Vec::new();

    for hook in hooks {
        if !hook.is_enabled(&running_data) {
            results.push(HookPlanEntry {
                key: hook.key.clone(),
                command: hook.command.display_argv(),
                should_run: false,
                skip_reason: Some("user_disabled".to_string()),
                template_errors: vec![],
            });
            continue;
        }

        if !hook.is_satisfied(&items, &running_data) {
            results.push(HookPlanEntry {
                key: hook.key.clone(),
                command: hook.command.display_argv(),
                should_run: false,
                skip_reason: Some("unsatisfied_needs".to_string()),
                template_errors: vec![],
            });
            continue;
        }

        // Evaluate conditional using the running context (includes
        // hook_ran_* for prior hooks).
        match hook.evaluate_conditional(&running_data) {
            Ok(false) => {
                results.push(HookPlanEntry {
                    key: hook.key.clone(),
                    command: hook.command.display_argv(),
                    should_run: false,
                    skip_reason: Some("false_conditional".to_string()),
                    template_errors: vec![],
                });
                continue;
            }
            Err(e) => {
                results.push(HookPlanEntry {
                    key: hook.key.clone(),
                    command: hook.command.display_argv(),
                    should_run: false,
                    skip_reason: Some(format!("conditional_error: {}", e)),
                    template_errors: vec![],
                });
                continue;
            }
            Ok(true) => {}
        }

        // Template the command into its `bash -c` body. Matches native
        // semantics: templating failure is a hard error — the hook is NOT
        // runnable, and hook_ran_* is NOT flipped (downstream conditionals
        // see false).
        let context = match Context::from_serialize(&running_data) {
            Ok(c) => c,
            Err(e) => {
                results.push(HookPlanEntry {
                    key: hook.key.clone(),
                    command: hook.command.display_argv(),
                    should_run: false,
                    skip_reason: Some("template_error".to_string()),
                    template_errors: vec![format!("context error: {}", e)],
                });
                continue;
            }
        };

        let argv = match render_command(&hook.command, &context) {
            Ok(argv) => argv,
            Err(e) => {
                results.push(HookPlanEntry {
                    key: hook.key.clone(),
                    command: hook.command.display_argv(),
                    should_run: false,
                    skip_reason: Some("template_error".to_string()),
                    template_errors: vec![format!("hook '{}': {}", hook.key, e)],
                });
                continue;
            }
        };

        // Refuse catastrophic patterns in the rendered `-c` body (argv[2]).
        // Treated like a template error: the hook does not run and hook_ran_*
        // stays false.
        if let Err(e) = dangerous_pattern_check(&argv[2]) {
            results.push(HookPlanEntry {
                key: hook.key.clone(),
                command: argv,
                should_run: false,
                skip_reason: Some("template_error".to_string()),
                template_errors: vec![format!("hook '{}': {}", hook.key, e)],
            });
            continue;
        }

        results.push(HookPlanEntry {
            key: hook.key.clone(),
            command: argv,
            should_run: true,
            skip_reason: None,
            template_errors: vec![],
        });

        // Mark this hook as "ran" for subsequent conditionals.
        running_data.insert(format!("hook_ran_{}", hook.key), "true".to_string());
    }

    results
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Error, Debug)]
pub enum Error {
    #[error("Error initializing runtime: {0}")]
    ErrorInitializingRuntime(io::Error),
    #[error("Error rendering template: {0}")]
    ErrorRenderingTemplate(Hook, tera::Error),
    #[error("Invalid conditional: {0}")]
    InvalidConditional(Hook, ConditionalError),
    #[error("Setup failed: {0}")]
    SetupFailed(Hook, io::Error),
    #[error("invalid hook command for '{key}': {source}", key = .0.key, source = .1)]
    InvalidHookCommand(Hook, #[source] HookCommandError),
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Debug)]
pub enum HookStreamResult {
    HookStarted(String),
    HookDone(HookResult),
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_hooks_stream(
    dir: impl AsRef<Path>,
    hooks: &Vec<Hook>,
    slots: &Vec<Slot>,
    data: &HashMap<String, String>,
    run_as_user: Option<User>,
) -> Result<impl Stream<Item = HookStreamResult>, Error> {
    let mut skipped_hooks = Vec::new();
    let mut queued_hooks = Vec::new();

    let items: Vec<&dyn Needy> = {
        let mut items = slots
            .iter()
            .map(|s| s as &dyn Needy)
            .collect::<Vec<&dyn Needy>>();
        items.extend(hooks.iter().map(|h| h as &dyn Needy));
        items
    };

    for hook in hooks {
        if hook.is_enabled(data) && hook.is_satisfied(&items, data) {
            queued_hooks.push(hook.clone());
        } else if hook.is_enabled(data) {
            skipped_hooks.push((hook.clone(), SkipReason::FalseConditional));
        } else {
            skipped_hooks.push((hook.clone(), SkipReason::UserDisabled));
        }
    }

    // Render each queued hook into its full `bash -c` argv, then build the
    // command to run it. Rendering happens here (with the initial slot data);
    // chained conditionals are re-evaluated in-stream against accumulated
    // hook_ran_* state.
    let mut commands = Vec::new();
    for hook in queued_hooks {
        let context = Context::from_serialize(data)
            .map_err(|e| Error::ErrorRenderingTemplate(hook.clone(), e))?;

        let argv = render_command(&hook.command, &context)
            .map_err(|e| Error::ErrorRenderingTemplate(hook.clone(), e))?;

        // argv[2] is the executable `-c` body — the thing the shell runs.
        dangerous_pattern_check(&argv[2])
            .map_err(|e| Error::InvalidHookCommand(hook.clone(), e))?;

        let cmd = match run_as_user {
            // TODO spackle shouldn't need to depend on polyjuice, it should instead be able to receive an arbitrary Command from a consumer, who may choose to wrap it in polyjuice or not
            Some(ref user) => match polyjuice::cmd_as_user(&argv[0], user.clone()) {
                Ok(cmd) => cmd,
                Err(e) => {
                    return Err(Error::SetupFailed(
                        hook.clone(),
                        io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to run command as user: {}", e),
                        ),
                    )); //TODO we probably want a different error type here
                }
            },
            None => process::Command::new(&argv[0]),
        };

        commands.push((hook, argv, async_process::Command::from(cmd)));
    }

    let slot_data_owned = data.clone();
    let hook_keys = hooks.iter().map(|h| h.key.clone()).collect::<Vec<String>>();

    Ok(stream! {
        for (hook, reason) in skipped_hooks {
            yield HookStreamResult::HookStarted(hook.key.clone());
            yield HookStreamResult::HookDone(HookResult {
                hook: hook.clone(),
                kind: HookResultKind::Skipped(reason),
            });
        }

        let mut ran_hooks = Vec::new();
        for (hook, argv, mut cmd) in commands {
            yield HookStreamResult::HookStarted(hook.key.clone());

            // Evaluate conditional
            // also add to the context the run status of all hooks so far
            // TODO this can be evaluated outside of stream once "needs" is implemented
            let mut cond_context = slot_data_owned.clone();
            for hook in &hook_keys {
                cond_context.insert(format!("hook_ran_{}", hook), "false".to_string());
            }
            for hook in ran_hooks.clone() {
                cond_context.insert(format!("hook_ran_{}", hook), "true".to_string());
            }

            let condition = match hook.evaluate_conditional(&cond_context) {
                Ok(condition) => condition,
                Err(e) => {
                    yield HookStreamResult::HookDone(HookResult {
                        hook: hook.clone(),
                        kind: HookResultKind::Failed(HookError::ConditionalFailed(e)),
                    });
                    continue;
                }
            };

            if !condition {
                yield HookStreamResult::HookDone(HookResult {
                    hook: hook.clone(),
                    kind: HookResultKind::Skipped(SkipReason::FalseConditional),
                });
                continue;
            }

            let cmd_result = cmd.args(&argv[1..])
                .current_dir(dir.as_ref())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output().await;

            let output = match cmd_result {
                Ok(output) => output,
                Err(e) => {
                    yield HookStreamResult::HookDone(HookResult {
                        hook: hook.clone(),
                        kind: HookResultKind::Failed(HookError::CommandLaunchFailed(e)),
                    });
                    continue;
                }
            };

            if !output.status.success() {
                yield HookStreamResult::HookDone(HookResult {
                    hook: hook.clone(),
                    kind: HookResultKind::Failed(HookError::CommandExited {
                        exit_code: output.status.code().unwrap_or(1),
                        stdout: output.stdout,
                        stderr: output.stderr,
                    }),
                });
                continue;
            }

            ran_hooks.push(hook.key.clone());

            yield HookStreamResult::HookDone(HookResult {
                hook: hook.clone(),
                kind: HookResultKind::Completed {
                    stdout: output.stdout,
                    stderr: output.stderr,
                }
            });
        }
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_hooks(
    hooks: &Vec<Hook>,
    dir: impl AsRef<Path>,
    slots: &Vec<Slot>,
    data: &HashMap<String, String>,
    run_as_user: Option<User>,
) -> Result<Vec<HookResult>, Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Error::ErrorInitializingRuntime)?;

    let results = runtime.block_on(async {
        let stream = run_hooks_stream(dir, hooks, slots, data, run_as_user)?;
        pin!(stream);

        let mut hook_results = Vec::new();

        while let Some(result) = stream.next().await {
            match result {
                HookStreamResult::HookStarted(_) => {}
                HookStreamResult::HookDone(hook_result) => {
                    hook_results.push(hook_result);
                }
            }
        }

        Ok(hook_results)
    })?;

    Ok(results)
}

/// Config-level hook error reported by [`validate_config`]. Distinct
/// from [`ValidateError`] (which is about runtime hook *data* — user
/// toggles) and from [`Error`] (which is about hook execution).
///
/// `hook_key` always identifies the offending hook. `span` is optional —
/// command-template parse errors carry a best-effort line/col from Tera;
/// reference errors don't have one.
#[derive(Debug, Clone)]
pub struct ConfigError {
    pub hook_key: String,
    pub message: String,
    pub span: Option<crate::diagnostic::Span>,
    pub code: Option<&'static str>,
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "hook '{}': {}", self.hook_key, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Validate hook configuration *statically* — no slot data needed. Catches:
///
///   - `needs` references that don't resolve to a known slot or hook key
///   - `if` conditional templates that fail to parse (unclosed brackets,
///     bad syntax)
///   - `command` part templates that fail to parse
///   - `command` bodies that match a blocked dangerous pattern (see
///     [`dangerous_pattern_check`])
///
/// Returns every problem found, not just the first. The companion to
/// `slot::validate`, used by the top-level `check` to produce structured
/// hook-config diagnostics.
pub fn validate_config(hooks: &[Hook], slots: &[Slot]) -> Vec<ConfigError> {
    let mut errors = Vec::new();
    let known_keys: std::collections::HashSet<&str> = slots
        .iter()
        .map(|s| s.key.as_str())
        .chain(hooks.iter().map(|h| h.key.as_str()))
        .collect();

    for hook in hooks {
        for needed in &hook.needs {
            if !known_keys.contains(needed.as_str()) {
                errors.push(ConfigError {
                    hook_key: hook.key.clone(),
                    message: format!("depends on unknown key '{}' (no such slot or hook)", needed),
                    span: None,
                    code: Some("hook::unknown_needs"),
                });
            }
        }

        // Parse the conditional template (parse-only — no values needed).
        if let Some(cond) = &hook.r#if {
            if let Err(e) = tera::Tera::default().add_raw_template("__hook_if__", cond) {
                let span = crate::diagnostic::extract_tera_span(&e);
                errors.push(ConfigError {
                    hook_key: hook.key.clone(),
                    message: format!("invalid `if` template: {}", e),
                    span,
                    code: Some("hook::if_template_parse"),
                });
            }
        }

        // Parse each command part's template. Parse-only catches unclosed
        // brackets / bad filter syntax without needing slot values.
        for (i, part) in hook.command.template_parts().iter().enumerate() {
            if let Err(e) = tera::Tera::default().add_raw_template("__hook_cmd__", part) {
                let span = crate::diagnostic::extract_tera_span(&e);
                errors.push(ConfigError {
                    hook_key: hook.key.clone(),
                    message: format!("invalid command part[{}] template: {}", i, e),
                    span,
                    code: Some("hook::command_template_parse"),
                });
            }
        }

        // Catch catastrophic patterns the author wrote literally. Slot-injected
        // dangers are caught post-template at plan/exec time; this is the
        // best-effort static check `spackle check` can do without slot data.
        if let Err(e) = dangerous_pattern_check(&hook.command.raw_body()) {
            errors.push(ConfigError {
                hook_key: hook.key.clone(),
                message: e.to_string(),
                span: None,
                code: Some("hook::command_dangerous_pattern"),
            });
        }
    }

    errors
}

#[derive(Serialize, Debug)]
pub enum ValidateError {
    UnknownKey(String),
    NotABoolean(String),
}

impl Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidateError::UnknownKey(key) => write!(f, "unknown key: {}", key),
            ValidateError::NotABoolean(key) => write!(f, "not a boolean: {}", key),
        }
    }
}

pub fn validate_data(
    data: &HashMap<String, String>,
    hooks: &Vec<Hook>,
) -> Result<(), ValidateError> {
    for entry in data.iter() {
        if !hooks.iter().any(|hook| hook.key == *entry.0) {
            return Err(ValidateError::UnknownKey(entry.0.clone()));
        }

        if entry.1.parse::<bool>().is_err() {
            return Err(ValidateError::NotABoolean(entry.0.clone()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::slot::SlotType;

    use super::*;

    #[test]
    fn basic() {
        let hooks = vec![Hook {
            key: "hello world".to_string(),
            command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
            ..Hook::default()
        }];

        assert!(run_hooks(&hooks, ".", &Vec::new(), &HashMap::new(), None).is_ok());
    }

    #[test]
    fn command_fail() {
        let hooks = vec![
            // Hook::new("okay".to_string(), vec!["true".to_string()]),
            Hook {
                key: "error".to_string(),
                command: HookCommand::Array(vec!["false".to_string()]),
                ..Hook::default()
            },
        ];

        let results = run_hooks(&hooks, ".", &Vec::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        assert!(
            results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Failed { .. },
                ..
            } if hook.key == "error")),
            "Expected error hook to fail, got {:?}",
            results
        );
    }

    #[test]
    fn error_executing() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: HookCommand::Array(vec!["invalid_cmd".to_string()]),
                ..Hook::default()
            },
        ];

        let results = run_hooks(&hooks, ".", &Vec::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        assert!(results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Completed { .. },
                ..
            } if hook.key == "1")));

        assert!(results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Failed { .. },
                ..
            } if hook.key == "2")));
    }

    #[test]
    fn conditional() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                r#if: Some("true".to_string()),
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                r#if: Some("false".to_string()),
                ..Hook::default()
            },
            Hook {
                key: "3".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                ..Hook::default()
            },
            Hook {
                key: "4".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                r#if: Some("{{ hook_ran_1 }}".to_string()),
                ..Hook::default()
            },
        ];

        let results = run_hooks(&hooks, ".", &Vec::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        let skipped_hooks: Vec<_> = results
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    HookResult {
                        kind: HookResultKind::Skipped { .. },
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(skipped_hooks.len(), 1);

        assert!(
            results.iter().any(|x| matches!(x, HookResult {
            hook,
            kind: HookResultKind::Completed { .. },
            ..
        } if hook.key == "4")),
            "Expected hook 4 to be completed, got {:?}",
            results
        );
    }

    #[test]
    fn bad_conditional_template() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                r#if: Some("{{ good_var }}".to_string()),
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                r#if: Some("{{ bad_var }}".to_string()),
                ..Hook::default()
            },
        ];

        let results = run_hooks(
            &hooks,
            ".",
            &Vec::new(),
            &HashMap::from([("good_var".to_string(), "true".to_string())]),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert!(results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Completed { .. },
                ..
            } if hook.key == "1")));

        assert!(results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Failed { .. },
                ..
            } if hook.key == "2")));
    }

    #[test]
    fn bad_conditional_value() {
        let hooks = vec![Hook {
            key: "1".to_string(),
            command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
            r#if: Some("lorem ipsum".to_string()),
            ..Hook::default()
        }];

        let results = run_hooks(
            &hooks,
            ".",
            &Vec::new(),
            &HashMap::from([("".to_string(), "".to_string())]),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert!(results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Failed { .. },
                ..
            } if hook.key == "1")));
    }

    #[test]
    fn optional() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                default: Some(false),
                ..Hook::default()
            },
            Hook {
                key: "3".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "hello world".to_string()]),
                default: Some(false),
                ..Hook::default()
            },
        ];

        let results = run_hooks(
            &hooks,
            ".",
            &Vec::new(),
            &HashMap::from([("3".to_string(), "true".to_string())]),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert_eq!(
            results.len(),
            3,
            "Expected 3 results, got {:?}",
            results.len()
        );

        assert!(results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Completed { .. },
                ..
            } if hook.key == "1")));

        assert!(results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Skipped { .. },
                ..
            } if hook.key == "2")));

        assert!(results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Completed { .. },
                ..
            } if hook.key == "3")));
    }

    #[test]
    fn templated_cmd() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: HookCommand::Array(vec![
                    "{{ field_1 }}".to_string(),
                    "{{ field_2 }}".to_string(),
                ]),
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: HookCommand::Array(vec![
                    "echo".to_string(),
                    "{{ _output_name }}".to_string(),
                ]),
                ..Hook::default()
            },
        ];

        let results = run_hooks(
            &hooks,
            ".",
            &Vec::new(),
            &HashMap::from([
                ("field_1".to_string(), "echo".to_string()),
                ("field_2".to_string(), "test".to_string()),
                ("_output_name".to_string(), "spackle".to_string()),
            ]),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert!(
            results.iter().all(|x| matches!(
                x,
                HookResult {
                    kind: HookResultKind::Completed { .. },
                    ..
                }
            )),
            "Expected all hooks to be completed, but got: {:?}",
            results
        );

        assert!(
            results.iter().any(|x| match x {
                HookResult {
                    hook,
                    kind: HookResultKind::Completed { stdout, .. },
                    ..
                } if hook.key == "2" => String::from_utf8_lossy(stdout).trim() == "spackle",
                _ => false,
            }),
            "Hook 'echo' should output 'test', got {:?}",
            results.iter().find(|x| x.hook.key == "echo")
        );
    }

    #[test]
    fn invalid_templated_cmd() {
        let hooks = vec![Hook {
            key: "1".to_string(),
            command: HookCommand::Array(vec![
                "{{ field_1 }}".to_string(),
                "{{ field_2 }}".to_string(),
            ]),
            ..Hook::default()
        }];

        let results = run_hooks(
            &hooks,
            ".",
            &Vec::new(),
            &HashMap::from([("field_1".to_string(), "echo".to_string())]),
            None,
        )
        .expect_err("run_hooks succeeded, should have failed");

        match results {
            Error::ErrorRenderingTemplate(_, _) => {}
            _ => panic!("Expected Error::ErrorRenderingTemplate, got {:?}", results),
        }
    }

    #[test]
    fn needs_satisfied_multi() {
        let hooks = vec![
            Hook {
                key: "hook".to_string(),
                command: HookCommand::Array(vec!["true".to_string()]),
                ..Hook::default()
            },
            Hook {
                key: "needy".to_string(),
                command: HookCommand::Array(vec!["true".to_string()]),
                needs: vec![
                    "hook".to_string(),
                    "string_slot".to_string(),
                    "number_slot".to_string(),
                    "bool_slot".to_string(),
                ],
                ..Hook::default()
            },
        ];

        let results = run_hooks(
            &hooks,
            ".",
            &Vec::from([
                Slot {
                    key: "string_slot".to_string(),
                    r#type: SlotType::String,
                    ..Default::default()
                },
                Slot {
                    key: "number_slot".to_string(),
                    r#type: SlotType::Number,
                    ..Default::default()
                },
                Slot {
                    key: "bool_slot".to_string(),
                    r#type: SlotType::Boolean,
                    ..Default::default()
                },
            ]),
            &HashMap::from([
                ("string_slot".to_string(), "foo".to_string()),
                ("number_slot".to_string(), "1".to_string()),
                ("bool_slot".to_string(), "true".to_string()),
            ]),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert!(
            results.iter().any(|x| matches!(x, HookResult {
            hook,
            kind: HookResultKind::Completed { .. },
            ..
        } if hook.key == "needy")),
            "Expected hook 'needy' to be completed, got {:?}",
            results.iter().find(|x| x.hook.key == "needy")
        );
    }

    #[test]
    fn needs_unsatisfied() {
        let hooks = vec![
            Hook {
                key: "hook".to_string(),
                command: HookCommand::Array(vec!["true".to_string()]),
                default: Some(false),
                ..Hook::default()
            },
            Hook {
                key: "needy".to_string(),
                command: HookCommand::Array(vec!["true".to_string()]),
                needs: vec!["hook".to_string()],
                ..Hook::default()
            },
        ];

        let results = run_hooks(&hooks, ".", &Vec::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        assert!(
            results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Skipped { .. },
                ..
            } if hook.key == "needy")),
            "Expected hook 'needy' to be skipped, got {:?}",
            results
        );
    }

    #[test]
    fn needs_invalid_key() {
        let hooks = vec![Hook {
            key: "hook".to_string(),
            command: HookCommand::Array(vec!["true".to_string()]),
            needs: vec!["invalid_key".to_string()],
            ..Hook::default()
        }];

        let results = run_hooks(&hooks, ".", &Vec::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        assert!(
            results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Skipped { .. },
                ..
            } if hook.key == "hook")),
            "Expected hook 'hook' to be skipped, got {:?}",
            results
        );
    }

    #[test]
    fn needs_transitive() {
        let hooks = vec![
            Hook {
                key: "a".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "a".to_string()]),
                ..Hook::default()
            },
            Hook {
                key: "b".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "b".to_string()]),
                needs: vec!["a".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "c".to_string(),
                command: HookCommand::Array(vec!["echo".to_string(), "c".to_string()]),
                needs: vec!["b".to_string()],
                ..Hook::default()
            },
        ];

        let results = run_hooks(&hooks, ".", &Vec::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        assert!(
            results.iter().any(|result| {
                matches!(result, HookResult {
                hook: Hook { key, .. },
                kind: HookResultKind::Completed { .. },
                ..
            } if key == "c")
            }),
            "Expected hook 'c' to be completed, got {:?}",
            results.iter().find(|x| x.hook.key == "c")
        );
    }

    #[test]
    fn needs_transitive_unsatisfied() {
        let hooks = vec![
            Hook {
                key: "hook_a".to_string(),
                command: HookCommand::Array(vec!["true".to_string()]),
                default: Some(false),
                needs: vec!["slot_a".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "hook_b".to_string(),
                command: HookCommand::Array(vec!["true".to_string()]),
                needs: vec!["hook_a".to_string()],
                ..Hook::default()
            },
        ];

        let results = run_hooks(
            &hooks,
            ".",
            &Vec::new(),
            &HashMap::from([("slot_a".to_string(), "false".to_string())]),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert!(
            results.iter().any(|x| matches!(x, HookResult {
                hook,
                kind: HookResultKind::Skipped { .. },
                ..
            } if hook.key == "hook_b")),
            "Expected hook 'hook_b' to be skipped, got {:?}",
            results.iter().find(|x| x.hook.key == "hook_b")
        );
    }

    #[test]
    fn test_validate_data_non_boolean() {
        let data = HashMap::from([("hook_a".to_string(), "foo".to_string())]);

        let hooks = Vec::from([Hook {
            key: "hook_a".to_string(),
            default: Some(false),
            ..Hook::default()
        }]);

        validate_data(&data, &hooks).expect_err("validate_data should have failed");
    }

    #[test]
    fn test_validate_data_missing_key() {
        let data = HashMap::from([("hook_a".to_string(), "true".to_string())]);

        let hooks = Vec::new();

        validate_data(&data, &hooks).expect_err("validate_data should have failed");
    }

    // --- Table-driven tests for evaluate_hook_plan ---

    #[test]
    fn evaluate_hook_plan_table() {
        use crate::slot::Slot;

        struct Case {
            name: &'static str,
            hooks: Vec<Hook>,
            slots: Vec<Slot>,
            data: Vec<(&'static str, &'static str)>,
            // (key, should_run, skip_reason_contains)
            expected: Vec<(&'static str, bool, Option<&'static str>)>,
        }

        let cases = vec![
            Case {
                name: "default=true → runs",
                hooks: vec![Hook {
                    key: "h1".to_string(),
                    command: HookCommand::Array(vec!["echo".to_string(), "hi".to_string()]),
                    default: Some(true),
                    ..Default::default()
                }],
                slots: vec![],
                data: vec![],
                expected: vec![("h1", true, None)],
            },
            Case {
                name: "default=false → user_disabled",
                hooks: vec![Hook {
                    key: "h1".to_string(),
                    command: HookCommand::Array(vec!["echo".to_string()]),
                    default: Some(false),
                    ..Default::default()
                }],
                slots: vec![],
                data: vec![],
                expected: vec![("h1", false, Some("user_disabled"))],
            },
            Case {
                name: "user override enables disabled hook",
                hooks: vec![Hook {
                    key: "h1".to_string(),
                    command: HookCommand::Array(vec!["echo".to_string()]),
                    default: Some(false),
                    ..Default::default()
                }],
                slots: vec![],
                data: vec![("h1", "true")],
                expected: vec![("h1", true, None)],
            },
            Case {
                name: "hook_ran injection: second hook conditional passes",
                hooks: vec![
                    Hook {
                        key: "first".to_string(),
                        command: HookCommand::Array(vec!["echo".to_string(), "1".to_string()]),
                        default: Some(true),
                        ..Default::default()
                    },
                    Hook {
                        key: "second".to_string(),
                        command: HookCommand::Array(vec!["echo".to_string(), "2".to_string()]),
                        r#if: Some("{{ hook_ran_first }}".to_string()),
                        default: Some(true),
                        ..Default::default()
                    },
                ],
                slots: vec![],
                data: vec![],
                expected: vec![("first", true, None), ("second", true, None)],
            },
            Case {
                name: "hook_ran injection: disabled first → second conditional false",
                hooks: vec![
                    Hook {
                        key: "first".to_string(),
                        command: HookCommand::Array(vec!["echo".to_string()]),
                        default: Some(false),
                        ..Default::default()
                    },
                    Hook {
                        key: "second".to_string(),
                        command: HookCommand::Array(vec!["echo".to_string()]),
                        r#if: Some("{{ hook_ran_first }}".to_string()),
                        default: Some(true),
                        ..Default::default()
                    },
                ],
                slots: vec![],
                data: vec![],
                expected: vec![
                    ("first", false, Some("user_disabled")),
                    ("second", false, Some("false_conditional")),
                ],
            },
            Case {
                name: "command templating with slot data",
                hooks: vec![Hook {
                    key: "h1".to_string(),
                    command: HookCommand::Array(vec![
                        "echo".to_string(),
                        "Hello {{ name }}".to_string(),
                    ]),
                    default: Some(true),
                    ..Default::default()
                }],
                slots: vec![],
                data: vec![("name", "world")],
                expected: vec![("h1", true, None)],
            },
            Case {
                name: "command template error → should_run=false + template_error skip",
                hooks: vec![Hook {
                    key: "broken".to_string(),
                    command: HookCommand::Array(vec![
                        "echo".to_string(),
                        "{{ undefined_var }}".to_string(),
                    ]),
                    default: Some(true),
                    ..Default::default()
                }],
                slots: vec![],
                data: vec![],
                expected: vec![("broken", false, Some("template_error"))],
            },
            Case {
                name: "template error blocks downstream hook_ran",
                hooks: vec![
                    Hook {
                        key: "broken".to_string(),
                        command: HookCommand::Array(vec!["{{ undefined }}".to_string()]),
                        default: Some(true),
                        ..Default::default()
                    },
                    Hook {
                        key: "after".to_string(),
                        command: HookCommand::Array(vec!["echo".to_string()]),
                        r#if: Some("{{ hook_ran_broken }}".to_string()),
                        default: Some(true),
                        ..Default::default()
                    },
                ],
                slots: vec![],
                data: vec![],
                expected: vec![
                    ("broken", false, Some("template_error")),
                    ("after", false, Some("false_conditional")),
                ],
            },
        ];

        for c in cases {
            let data: HashMap<String, String> = c
                .data
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            let plan = evaluate_hook_plan(&c.hooks, &c.slots, &data);

            assert_eq!(plan.len(), c.expected.len(), "case {}: plan length", c.name);

            for (entry, (exp_key, exp_run, exp_skip)) in plan.iter().zip(c.expected.iter()) {
                assert_eq!(entry.key, *exp_key, "case {}: key", c.name);
                assert_eq!(
                    entry.should_run, *exp_run,
                    "case {}: should_run for {}",
                    c.name, exp_key
                );
                match exp_skip {
                    Some(needle) => {
                        let reason = entry.skip_reason.as_deref().unwrap_or("");
                        assert!(
                            reason.contains(needle),
                            "case {}: skip_reason for {} should contain {:?}, got {:?}",
                            c.name,
                            exp_key,
                            needle,
                            reason,
                        );
                    }
                    None => assert!(
                        entry.skip_reason.is_none(),
                        "case {}: {} should have no skip_reason, got {:?}",
                        c.name,
                        exp_key,
                        entry.skip_reason,
                    ),
                }
            }

            // Extra assertions for specific cases
            if c.name == "command templating with slot data" {
                assert_eq!(
                    plan[0].command,
                    vec!["bash", "-c", "echo 'Hello world'"],
                    "case {}: templated command",
                    c.name
                );
                assert!(plan[0].template_errors.is_empty());
            }
            if c.name == "command template error → should_run=false + template_error skip" {
                assert!(
                    !plan[0].template_errors.is_empty(),
                    "case {}: should have template_errors",
                    c.name,
                );
            }
            if c.name == "template error blocks downstream hook_ran" {
                // First hook failed templating → not run → hook_ran_broken stays false
                assert!(!plan[0].template_errors.is_empty());
                // Second hook's conditional evaluates {{ hook_ran_broken }} = false → skip
                assert!(plan[1].template_errors.is_empty());
            }
        }
    }

    // --- HookCommand rendering + denylist ---

    fn vs(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    fn arr(args: &[&str]) -> HookCommand {
        HookCommand::Array(vs(args))
    }

    fn ctx(data: &HashMap<String, String>) -> Context {
        Context::from_serialize(data).expect("context")
    }

    #[test]
    fn render_command_table() {
        let empty = HashMap::new();
        let name_hi = HashMap::from([("name".to_string(), "hi".to_string())]);
        let name_meta = HashMap::from([("name".to_string(), "a; b".to_string())]);

        struct Case {
            name: &'static str,
            cmd: HookCommand,
            data: HashMap<String, String>,
            // Full expected argv (always begins with bash, -c).
            expected: Vec<&'static str>,
        }

        let cases = vec![
            Case {
                name: "string raw",
                cmd: HookCommand::String("echo hi && echo done".to_string()),
                data: empty.clone(),
                expected: vec!["bash", "-c", "echo hi && echo done"],
            },
            Case {
                name: "string raw substitution",
                cmd: HookCommand::String("echo {{ name }} && echo done".to_string()),
                data: name_hi.clone(),
                expected: vec!["bash", "-c", "echo hi && echo done"],
            },
            Case {
                name: "array simple",
                cmd: arr(&["echo", "hi"]),
                data: empty.clone(),
                expected: vec!["bash", "-c", "echo hi"],
            },
            Case {
                name: "array whitespace gets quoted",
                cmd: arr(&["echo", "hello world"]),
                data: empty.clone(),
                expected: vec!["bash", "-c", "echo 'hello world'"],
            },
            Case {
                name: "array operators pass through",
                cmd: arr(&["git", "init", "&&", "git", "add", "."]),
                data: empty.clone(),
                expected: vec!["bash", "-c", "git init && git add ."],
            },
            Case {
                name: "array template is literal",
                cmd: arr(&["echo", "{{ name }}", "&&", "echo", "done"]),
                data: name_hi.clone(),
                expected: vec!["bash", "-c", "echo hi && echo done"],
            },
            Case {
                name: "array template with metachars stays literal",
                cmd: arr(&["echo", "{{ name }}"]),
                data: name_meta.clone(),
                expected: vec!["bash", "-c", "echo 'a; b'"],
            },
            Case {
                name: "bash -c passthrough templates body",
                cmd: arr(&["bash", "-c", "echo {{ name }} && echo done"]),
                data: name_hi.clone(),
                expected: vec!["bash", "-c", "echo hi && echo done"],
            },
            Case {
                name: "sh -c passthrough",
                cmd: arr(&["sh", "-c", "x && y"]),
                data: empty.clone(),
                expected: vec!["bash", "-c", "x && y"],
            },
            Case {
                name: "bash -c passthrough preserves positional args",
                cmd: arr(&["bash", "-c", "echo {{ name }}", "argzero"]),
                data: name_hi.clone(),
                expected: vec!["bash", "-c", "echo hi", "argzero"],
            },
        ];

        for c in cases {
            let got = render_command(&c.cmd, &ctx(&c.data)).expect(c.name);
            assert_eq!(got, c.expected, "case {}", c.name);
        }
    }

    #[test]
    fn dangerous_pattern_check_table() {
        // (input, expected_ok)
        let cases: Vec<(&str, bool)> = vec![
            ("echo hi", true),
            ("rm -rf build", true),
            ("rm -rf /tmp/foo", true),
            ("rm -r /", true), // recursive but not forced
            ("rm -f /", true), // forced but not recursive
            ("rm -rf /", false),
            ("rm -rf /*", false),
            ("rm -fr /", false),
            ("rm --recursive --force /", false),
            ("rm -rf /etc", false),
            ("rm -rf /usr/", false),
            ("sudo rm -rf / --no-preserve-root", false),
            ("doas rm -rf /", false),
            ("echo done && rm -rf /", false),
            ("true && sudo rm -rf /etc", false),
            (":(){ :|:& };:", false),
            (":(){:|:&};:", false),
            // Separators that are NOT whitespace-padded must still split the
            // body so the trailing `rm` is recognized as a command.
            ("echo safe; rm -rf /", false),
            ("echo safe;rm -rf /", false),
            ("echo safe&&rm -rf /", false),
            ("echo safe||rm -rf /", false),
            ("echo safe|rm -rf /", false),
            ("echo safe & rm -rf /", false),
            ("echo safe\nrm -rf /", false),
            // A quoted literal containing the text is data, not an executable
            // command — correctly NOT flagged, even with an inner separator.
            ("echo 'rm -rf /'", true),
            ("echo 'a; rm -rf /'", true),
            ("echo \"a; rm -rf /\"", true),
            // `rm` as an ARGUMENT to another command is not an rm invocation —
            // must NOT be flagged (array form promises literal arguments).
            ("echo rm -rf /", true),
            ("printf rm -rf /", true),
            ("git commit -m rm -rf /", true),
            // ...but a real rm after a separator still is.
            ("echo rm && rm -rf /", false),
        ];
        for (input, ok) in cases {
            assert_eq!(
                dangerous_pattern_check(input).is_ok(),
                ok,
                "input {:?}",
                input
            );
        }
    }

    #[test]
    fn evaluate_hook_plan_array_chain_wraps() {
        let hooks = vec![Hook {
            key: "chain".to_string(),
            command: arr(&["git", "init", "&&", "git", "add", "."]),
            default: Some(true),
            ..Hook::default()
        }];
        let plan = evaluate_hook_plan(&hooks, &[], &HashMap::new());
        assert_eq!(plan.len(), 1);
        assert!(plan[0].should_run);
        assert_eq!(
            plan[0].command,
            vs(&["bash", "-c", "git init && git add ."])
        );
        assert!(plan[0].template_errors.is_empty());
    }

    #[test]
    fn evaluate_hook_plan_array_template_operator_is_safe() {
        // Previously a hard collision error; now template-then-quote makes
        // the slot value a literal argument, so the hook runs and downstream
        // conditionals see it ran.
        let hooks = vec![
            Hook {
                key: "first".to_string(),
                command: arr(&["echo", "{{ name }}", "&&", "echo", "done"]),
                default: Some(true),
                ..Hook::default()
            },
            Hook {
                key: "after".to_string(),
                command: arr(&["echo", "after"]),
                r#if: Some("{{ hook_ran_first }}".to_string()),
                default: Some(true),
                ..Hook::default()
            },
        ];
        let data = HashMap::from([("name".to_string(), "evil; rm x".to_string())]);
        let plan = evaluate_hook_plan(&hooks, &[], &data);
        assert_eq!(plan.len(), 2);
        assert!(plan[0].should_run, "got {:?}", plan[0]);
        assert_eq!(
            plan[0].command,
            vs(&["bash", "-c", "echo 'evil; rm x' && echo done"])
        );
        assert!(plan[1].should_run);
    }

    #[test]
    fn evaluate_hook_plan_string_form_raw_substitution() {
        let hooks = vec![Hook {
            key: "s".to_string(),
            command: HookCommand::String("echo {{ name }} && echo done".to_string()),
            default: Some(true),
            ..Hook::default()
        }];
        let data = HashMap::from([("name".to_string(), "hi".to_string())]);
        let plan = evaluate_hook_plan(&hooks, &[], &data);
        assert!(plan[0].should_run);
        assert_eq!(plan[0].command, vs(&["bash", "-c", "echo hi && echo done"]));
    }

    #[test]
    fn evaluate_hook_plan_bash_c_passthrough() {
        let hooks = vec![Hook {
            key: "explicit".to_string(),
            command: arr(&["bash", "-c", "echo {{ name }} && echo done"]),
            default: Some(true),
            ..Hook::default()
        }];
        let data = HashMap::from([("name".to_string(), "hi".to_string())]);
        let plan = evaluate_hook_plan(&hooks, &[], &data);
        assert!(plan[0].should_run);
        assert_eq!(plan[0].command, vs(&["bash", "-c", "echo hi && echo done"]));
    }

    #[test]
    fn evaluate_hook_plan_bash_c_with_positionals_is_scanned() {
        // Regression: a `bash -c` array with extra positional args must NOT
        // bypass the denylist. The `-c` body (argv[2]) is rendered verbatim
        // and scanned, so a slot-injected `rm -rf /` is still caught.
        let hooks = vec![Hook {
            key: "sneaky".to_string(),
            command: arr(&["bash", "-c", "rm -rf {{ target }}", "x"]),
            default: Some(true),
            ..Hook::default()
        }];
        let data = HashMap::from([("target".to_string(), "/".to_string())]);
        let plan = evaluate_hook_plan(&hooks, &[], &data);
        assert!(!plan[0].should_run, "got {:?}", plan[0]);
        assert_eq!(plan[0].skip_reason.as_deref(), Some("template_error"));
        assert!(
            plan[0].template_errors[0].contains("dangerous"),
            "got {:?}",
            plan[0].template_errors
        );
    }

    #[test]
    fn evaluate_hook_plan_command_arg_named_rm_is_not_flagged() {
        // Array form promises literal arguments: `rm` passed to `echo` is
        // data, not a command, and must run.
        let hooks = vec![Hook {
            key: "echo_rm".to_string(),
            command: arr(&["echo", "rm", "-rf", "/"]),
            default: Some(true),
            ..Hook::default()
        }];
        let plan = evaluate_hook_plan(&hooks, &[], &HashMap::new());
        assert!(plan[0].should_run, "got {:?}", plan[0]);
        assert_eq!(plan[0].command, vs(&["bash", "-c", "echo rm -rf /"]));
    }

    #[test]
    fn evaluate_hook_plan_denylist_blocks() {
        let hooks = vec![
            Hook {
                key: "danger".to_string(),
                command: arr(&["rm", "-rf", "/"]),
                default: Some(true),
                ..Hook::default()
            },
            Hook {
                key: "after".to_string(),
                command: arr(&["echo", "after"]),
                r#if: Some("{{ hook_ran_danger }}".to_string()),
                default: Some(true),
                ..Hook::default()
            },
        ];
        let plan = evaluate_hook_plan(&hooks, &[], &HashMap::new());
        assert_eq!(plan.len(), 2);
        assert!(!plan[0].should_run);
        assert_eq!(plan[0].skip_reason.as_deref(), Some("template_error"));
        assert!(
            plan[0].template_errors[0].contains("dangerous"),
            "got {:?}",
            plan[0].template_errors
        );
        // hook_ran_danger stays false → downstream conditional fails
        assert!(!plan[1].should_run);
        assert_eq!(plan[1].skip_reason.as_deref(), Some("false_conditional"));
    }

    #[test]
    fn run_hooks_stream_array_chain_executes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let hooks = vec![Hook {
            key: "chain".to_string(),
            command: arr(&["touch", "a", "&&", "touch", "b"]),
            ..Hook::default()
        }];
        let results = run_hooks(&hooks, tmp.path(), &Vec::new(), &HashMap::new(), None)
            .expect("run_hooks failed");
        assert!(
            results.iter().any(|r| matches!(
                r,
                HookResult {
                    kind: HookResultKind::Completed { .. },
                    ..
                }
            )),
            "expected chained command to complete, got {:?}",
            results
        );
        assert!(tmp.path().join("a").exists(), "file 'a' not created");
        assert!(tmp.path().join("b").exists(), "file 'b' not created");
    }

    #[test]
    fn run_hooks_stream_string_form_executes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let hooks = vec![Hook {
            key: "s".to_string(),
            command: HookCommand::String("touch {{ name }} && touch other".to_string()),
            ..Hook::default()
        }];
        let results = run_hooks(
            &hooks,
            tmp.path(),
            &Vec::new(),
            &HashMap::from([("name".to_string(), "first".to_string())]),
            None,
        )
        .expect("run_hooks failed");
        assert!(results.iter().all(|r| matches!(
            r,
            HookResult {
                kind: HookResultKind::Completed { .. },
                ..
            }
        )));
        assert!(tmp.path().join("first").exists());
        assert!(tmp.path().join("other").exists());
    }

    #[test]
    fn run_hooks_stream_array_literal_substitution() {
        // A slot value with shell metacharacters becomes ONE literal argument
        // (a filename), not a second command.
        let tmp = tempfile::tempdir().expect("tempdir");
        let hooks = vec![Hook {
            key: "lit".to_string(),
            command: arr(&["touch", "{{ name }}"]),
            ..Hook::default()
        }];
        let evil = "weird; name";
        let results = run_hooks(
            &hooks,
            tmp.path(),
            &Vec::new(),
            &HashMap::from([("name".to_string(), evil.to_string())]),
            None,
        )
        .expect("run_hooks failed");
        assert!(results.iter().all(|r| matches!(
            r,
            HookResult {
                kind: HookResultKind::Completed { .. },
                ..
            }
        )));
        assert!(
            tmp.path().join(evil).exists(),
            "file with literal name should exist"
        );
        // No injection: the metachar split must NOT have created a bare 'name'.
        assert!(!tmp.path().join("name").exists());
    }

    #[test]
    fn run_hooks_stream_denylist_blocks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let hooks = vec![Hook {
            key: "danger".to_string(),
            command: HookCommand::String("rm -rf /".to_string()),
            ..Hook::default()
        }];
        let err = run_hooks(&hooks, tmp.path(), &Vec::new(), &HashMap::new(), None)
            .expect_err("expected InvalidHookCommand error");

        let msg = err.to_string();
        assert!(
            msg.contains("'danger'"),
            "expected hook key in msg, got: {}",
            msg
        );
        assert!(
            msg.contains("dangerous"),
            "expected denylist text, got: {}",
            msg
        );

        match err {
            Error::InvalidHookCommand(_, HookCommandError::DangerousPattern { .. }) => {}
            other => panic!(
                "expected InvalidHookCommand/DangerousPattern, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn validate_config_flags_dangerous_pattern() {
        let hooks = vec![
            Hook {
                key: "danger".to_string(),
                command: arr(&["rm", "-rf", "/"]),
                ..Hook::default()
            },
            Hook {
                key: "fine".to_string(),
                command: arr(&["echo", "hi"]),
                ..Hook::default()
            },
            Hook {
                key: "fine_chain".to_string(),
                command: arr(&["touch", "a", "&&", "touch", "b"]),
                ..Hook::default()
            },
            // Array literal containing an inner separator: posix-quoted at
            // runtime, so it's data, not a chained `rm`. The static scan must
            // mirror that and NOT flag it.
            Hook {
                key: "quoted_literal".to_string(),
                command: arr(&["echo", "safe; rm -rf /"]),
                ..Hook::default()
            },
        ];
        let errors = validate_config(&hooks, &[]);

        let flagged: Vec<_> = errors
            .iter()
            .filter(|e| e.code == Some("hook::command_dangerous_pattern"))
            .collect();
        assert_eq!(
            flagged.len(),
            1,
            "expected exactly one dangerous-pattern diag, got {:#?}",
            errors
        );
        assert_eq!(flagged[0].hook_key, "danger");
    }
}
