use super::slot::Slot;
use async_process::Stdio;
use async_stream::stream;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Display, path::Path};
use std::{io, process};
use tera::{Context, Tera};
use tokio::pin;
use tokio_stream::{Stream, StreamExt};
use users::User;

use crate::needs::{is_satisfied, Needy};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Hook {
    pub key: String,
    pub command: Vec<String>,
    pub r#if: Option<String>,
    pub optional: Option<HookConfigOptional>,
    #[serde(default)]
    pub needs: Vec<String>,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HookConfigOptional {
    pub default: bool,
}

impl Display for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}\n{}",
            self.key.bold(),
            if let Some(optional) = &self.optional {
                format!(
                    "optional, default {}",
                    if optional.default {
                        "on".green()
                    } else {
                        "off".red()
                    }
                )
            } else {
                "".to_string()
            }
            .dimmed(),
            self.command
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
                .join(" ")
                .dimmed()
        )
    }
}

impl Default for Hook {
    fn default() -> Self {
        Hook {
            key: "".to_string(),
            command: vec![],
            r#if: None,
            optional: None,
            needs: vec![],
            name: None,
            description: None,
        }
    }
}

impl Needy for Hook {
    fn key(&self) -> String {
        self.key.clone()
    }

    fn is_enabled(&self, data: &HashMap<String, String>) -> bool {
        match &self.optional {
            Some(optional) => data
                .get(&self.key)
                .map(|s: &String| s.parse::<bool>().unwrap_or(false))
                .unwrap_or(optional.default),
            None => true,
        }
    }

    fn is_satisfied(&self, items: &Vec<&dyn Needy>, data: &HashMap<String, String>) -> bool {
        is_satisfied(&self.needs, items, data)
    }
}

impl Hook {
    fn evaluate_conditional(
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

#[derive(Serialize, Debug)]
pub struct HookResult {
    pub hook: Hook,
    pub kind: HookResultKind,
}

#[derive(Serialize, Debug)]
pub enum HookResultKind {
    Skipped(SkipReason),
    Completed { stdout: Vec<u8>, stderr: Vec<u8> },
    Failed(HookError),
}

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

#[derive(Debug)]
pub enum Error {
    ErrorInitializingRuntime(io::Error),
    ErrorRenderingTemplate(Hook, tera::Error),
    InvalidConditional(Hook, ConditionalError),
    SetupFailed(Hook, io::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ErrorInitializingRuntime(e) => {
                write!(f, "error initializing runtime: {}", e)
            }
            Error::ErrorRenderingTemplate(hook, e) => {
                write!(f, "error rendering template for hook {}: {}", hook.key, e)
            }
            Error::InvalidConditional(hook, e) => {
                write!(f, "invalid conditional for hook {}: {}", hook.key, e)
            }
            Error::SetupFailed(hook, e) => {
                write!(f, "setup failed for hook {}: {}", hook.key, e)
            }
        }
    }
}

#[derive(Serialize, Debug)]
pub enum HookStreamResult {
    HookStarted(String),
    HookDone(HookResult),
}

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

    // Apply template to command
    let mut templated_hooks = Vec::new();
    for hook in queued_hooks {
        let context = Context::from_serialize(data.clone())
            .map_err(|e| Error::ErrorRenderingTemplate(hook.clone(), e))?;

        let command = hook
            .command
            .iter()
            .map(|arg| {
                Tera::one_off(arg, &context, false)
                    .map_err(|e| Error::ErrorRenderingTemplate(hook.clone(), e))
            })
            .collect::<Result<Vec<String>, Error>>()?;

        templated_hooks.push(Hook {
            command,
            ..hook.clone()
        });
    }

    let mut commands = Vec::new();
    for hook in templated_hooks {
        let cmd = match run_as_user {
            Some(ref user) => match polyjuice::cmd_as_user(&hook.command[0], user.clone()) {
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
            None => process::Command::new(&hook.command[0]),
        };

        commands.push((hook, async_process::Command::from(cmd)));
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
        for (hook, mut cmd) in commands {
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

            let cmd_result = cmd.args(&hook.command[1..])
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

#[derive(Serialize, Debug)]
pub enum ValidateError {
    UnknownKey(String),
    NotOptional(String),
    NotABoolean(String),
}

impl Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidateError::UnknownKey(key) => write!(f, "unknown key: {}", key),
            ValidateError::NotABoolean(key) => write!(f, "not a boolean: {}", key),
            ValidateError::NotOptional(key) => write!(f, "not optional: {}", key),
        }
    }
}

pub fn validate_data(
    data: &HashMap<String, String>,
    hooks: &Vec<Hook>,
) -> Result<(), ValidateError> {
    for entry in data.iter() {
        match hooks.iter().find(|hook| hook.key == *entry.0) {
            None => return Err(ValidateError::UnknownKey(entry.0.clone())),
            Some(hook) => {
                if hook.optional.is_none() {
                    return Err(ValidateError::NotOptional(entry.0.clone()));
                }
            }
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
            command: vec!["echo".to_string(), "hello world".to_string()],
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
                command: vec!["false".to_string()],
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
                command: vec!["echo".to_string(), "hello world".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: vec!["invalid_cmd".to_string()],
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
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: Some("true".to_string()),
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: Some("false".to_string()),
                ..Hook::default()
            },
            Hook {
                key: "3".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "4".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
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
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: Some("{{ good_var }}".to_string()),
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
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
            command: vec!["echo".to_string(), "hello world".to_string()],
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
                command: vec!["echo".to_string(), "hello world".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                optional: Some(HookConfigOptional { default: false }),
                ..Hook::default()
            },
            Hook {
                key: "3".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                optional: Some(HookConfigOptional { default: false }),
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
                command: vec!["{{ field_1 }}".to_string(), "{{ field_2 }}".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "2".to_string(),
                command: vec!["echo".to_string(), "{{ _project_name }}".to_string()],
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
            command: vec!["{{ field_1 }}".to_string(), "{{ field_2 }}".to_string()],
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
                command: vec!["true".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "needy".to_string(),
                command: vec!["true".to_string()],
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
                command: vec!["true".to_string()],
                optional: Some(HookConfigOptional { default: false }),
                ..Hook::default()
            },
            Hook {
                key: "needy".to_string(),
                command: vec!["true".to_string()],
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
            command: vec!["true".to_string()],
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
                command: vec!["echo".to_string(), "a".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "b".to_string(),
                command: vec!["echo".to_string(), "b".to_string()],
                needs: vec!["a".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "c".to_string(),
                command: vec!["echo".to_string(), "c".to_string()],
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
                command: vec!["true".to_string()],
                optional: Some(HookConfigOptional { default: false }),
                needs: vec!["slot_a".to_string()],
                ..Hook::default()
            },
            Hook {
                key: "hook_b".to_string(),
                command: vec!["true".to_string()],
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
            optional: Some(HookConfigOptional { default: false }),
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

    #[test]
    fn test_validate_data_not_optional() {
        let data = HashMap::from([("hook_a".to_string(), "true".to_string())]);

        let hooks = Vec::from([Hook {
            key: "hook_a".to_string(),
            ..Hook::default()
        }]);

        validate_data(&data, &hooks).expect_err("validate_data should have failed");
    }
}
