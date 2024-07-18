use std::{collections::HashMap, fmt::Display, path::Path};
use std::{io, process};

use async_process::Stdio;
use async_stream::stream;
use serde::Serialize;
use tera::{Context, Tera};
use tokio::pin;
use tokio_stream::{Stream, StreamExt};

use super::config::Hook;
use users::User;

#[derive(Serialize, Debug)]
pub enum HookResult {
    Skipped {
        hook: Hook,
        reason: SkipReason,
    },
    Completed {
        hook: Hook,
        stdout: String,
        stderr: String,
    },
    Failed {
        hook: Hook,
        error: HookError,
    },
}

#[derive(Serialize, Debug)]
pub enum HookError {
    ConditionalFailed(ConditionalError),
    CommandLaunchFailed(#[serde(skip)] io::Error),
    CommandExited(String),
}

impl Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookError::ConditionalFailed(e) => write!(f, "conditional failed: {}", e),
            HookError::CommandLaunchFailed(e) => write!(f, "command launch failed: {}", e),
            HookError::CommandExited(e) => write!(f, "command exited: {}", e),
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
    hooks: &Vec<Hook>,
    dir: impl AsRef<Path>,
    slot_data: &HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
    run_as_user: Option<User>,
) -> Result<impl Stream<Item = HookStreamResult>, Error> {
    let mut slot_data = slot_data.clone();
    slot_data.insert(
        "project_name".to_string(),
        dir.as_ref()
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(".".to_string()),
    );

    let mut skipped_hooks = Vec::new();
    let mut queued_hooks = Vec::new();

    for hook in hooks {
        let enabled = match &hook.optional {
            Some(optional) => hook_data.get(&hook.key).unwrap_or(&optional.default),
            None => &true,
        };
        if *enabled {
            queued_hooks.push(hook.clone());
        } else {
            skipped_hooks.push((hook.clone(), SkipReason::UserDisabled));
        }
    }

    // Apply template to command
    let mut templated_hooks = Vec::new();
    for hook in queued_hooks {
        let context = Context::from_serialize(slot_data.clone())
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

    let slot_data_owned = slot_data.clone();
    let hook_keys = hooks.iter().map(|h| h.key.clone()).collect::<Vec<String>>();

    Ok(stream! {
        for hook in skipped_hooks {
            yield HookStreamResult::HookDone(HookResult::Skipped {
                hook: hook.0,
                reason: hook.1,
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

            let condition = match evaluate_conditional(&hook, &slot_data_owned) {
                Ok(condition) => condition,
                Err(e) => {
                    yield HookStreamResult::HookDone(HookResult::Failed {
                        hook: hook.clone(),
                        error: HookError::ConditionalFailed(e),
                    });
                    continue;
                }
            };

            if !condition {
                yield HookStreamResult::HookDone(HookResult::Skipped {
                    hook: hook.clone(),
                    reason: SkipReason::FalseConditional,
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
                    yield HookStreamResult::HookDone(HookResult::Failed {
                        hook: hook.clone(),
                        error: HookError::CommandLaunchFailed(e),
                    });
                    continue;
                }
            };

            if !output.status.success() {
                yield HookStreamResult::HookDone(HookResult::Failed {
                    hook: hook.clone(),
                    error: HookError::CommandExited(String::from_utf8_lossy(&output.stderr).to_string()),
                });
                continue;
            }

            ran_hooks.push(hook.key.clone());

            yield HookStreamResult::HookDone(HookResult::Completed {
                hook: hook.clone(),
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
    })
}

pub fn run_hooks(
    hooks: &Vec<Hook>,
    dir: impl AsRef<Path>,
    slot_data: &HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
    run_as_user: Option<User>,
) -> Result<Vec<HookResult>, Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::ErrorInitializingRuntime(e))?;

    let results = runtime.block_on(async {
        let stream = run_hooks_stream(hooks, dir, slot_data, hook_data, run_as_user)?;
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

fn evaluate_conditional(
    hook: &Hook,
    context: &HashMap<String, String>,
) -> Result<bool, ConditionalError> {
    let conditional = match hook.clone().r#if {
        Some(conditional) => conditional,
        None => return Ok(true),
    };

    let context =
        Context::from_serialize(context).map_err(|e| ConditionalError::InvalidContext(e))?;

    let condition_str = Tera::one_off(&conditional, &context, false)
        .map_err(|e| ConditionalError::InvalidTemplate(e))?;

    let condition = condition_str
        .trim()
        .parse::<bool>()
        .map_err(|e| ConditionalError::NotBoolean(e.to_string()))?;

    Ok(condition)
}

#[cfg(test)]
mod tests {
    use crate::core::config::HookConfigOptional;

    use super::*;

    #[test]
    fn basic() {
        let hooks = vec![Hook {
            key: "hello world".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        }];

        assert!(run_hooks(&hooks, ".", &HashMap::new(), &HashMap::new(), None).is_ok());
    }

    #[test]
    fn command_fail() {
        let hooks = vec![
            Hook {
                key: "hello world".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: None,
                optional: None,
                name: None,
                description: None,
            },
            Hook {
                key: "error".to_string(),
                command: vec!["false".to_string()],
                r#if: None,
                optional: None,
                name: None,
                description: None,
            },
        ];

        let result = run_hooks(&hooks, ".", &HashMap::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        assert_eq!(result.len(), 2, "Expected 2 results, got {:?}", result);

        assert!(matches!(result[0], HookResult::Completed { .. }));
    }

    #[test]
    fn error_executing() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: None,
                optional: None,
                name: None,
                description: None,
            },
            Hook {
                key: "2".to_string(),
                command: vec!["invalid_cmd".to_string()],
                r#if: None,
                optional: None,
                name: None,
                description: None,
            },
        ];

        let results = run_hooks(&hooks, ".", &HashMap::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        assert!(results
            .iter()
            .any(|x| matches!(x, HookResult::Completed { hook, .. } if hook.key == "1")));

        assert!(results
            .iter()
            .any(|x| matches!(x, HookResult::Failed { hook, .. } if hook.key == "2")));
    }

    #[test]
    fn conditional() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: Some("true".to_string()),
                optional: None,
                name: None,
                description: None,
            },
            Hook {
                key: "2".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: Some("false".to_string()),
                optional: None,
                name: None,
                description: None,
            },
            Hook {
                key: "3".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: None,
                optional: None,
                name: None,
                description: None,
            },
            Hook {
                key: "4".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: Some("{{ hook_ran_1 }}".to_string()),
                optional: None,
                name: None,
                description: None,
            },
        ];

        let results = run_hooks(&hooks, ".", &HashMap::new(), &HashMap::new(), None)
            .expect("run_hooks failed, should have succeeded");

        let skipped_hooks: Vec<_> = results
            .iter()
            .filter(|r| matches!(r, HookResult::Skipped { .. }))
            .collect();
        assert_eq!(skipped_hooks.len(), 1);
    }

    #[test]
    fn bad_conditional_template() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: Some("{{ good_var }}".to_string()),
                optional: None,
                name: None,
                description: None,
            },
            Hook {
                key: "2".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: Some("{{ bad_var }}".to_string()),
                optional: None,
                name: None,
                description: None,
            },
        ];

        let results = run_hooks(
            &hooks,
            ".",
            &HashMap::from([("good_var".to_string(), "true".to_string())]),
            &HashMap::new(),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert!(results
            .iter()
            .any(|x| matches!(x, HookResult::Completed { hook, .. } if hook.key == "1")));

        assert!(results
            .iter()
            .any(|x| matches!(x, HookResult::Failed { hook, .. } if hook.key == "2")));
    }

    #[test]
    fn bad_conditional_value() {
        let hooks = vec![Hook {
            key: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("lorem ipsum".to_string()),
            optional: None,
            name: None,
            description: None,
        }];

        let results = run_hooks(
            &hooks,
            ".",
            &HashMap::from([("".to_string(), "".to_string())]),
            &HashMap::new(),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert!(results
            .iter()
            .any(|x| matches!(x, HookResult::Failed { hook, .. } if hook.key == "1")));
    }

    #[test]
    fn optional() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: None,
                optional: None,
                name: None,
                description: None,
            },
            Hook {
                key: "2".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: None,
                optional: Some(HookConfigOptional { default: false }),
                name: None,
                description: None,
            },
            Hook {
                key: "3".to_string(),
                command: vec!["echo".to_string(), "hello world".to_string()],
                r#if: None,
                optional: Some(HookConfigOptional { default: false }),
                name: None,
                description: None,
            },
        ];

        let results = run_hooks(
            &hooks,
            ".",
            &HashMap::new(),
            &HashMap::from([("3".to_string(), true)]),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert_eq!(
            results.len(),
            3,
            "Expected 3 results, got {:?}",
            results.len()
        );

        assert!(results
            .iter()
            .any(|x| matches!(x, HookResult::Completed { hook, .. } if hook.key == "1")));

        assert!(results
            .iter()
            .any(|x| matches!(x, HookResult::Skipped { hook, .. } if hook.key == "2")));

        assert!(results
            .iter()
            .any(|x| matches!(x, HookResult::Completed { hook, .. } if hook.key == "3")));
    }

    #[test]
    fn templated_cmd() {
        let hooks = vec![
            Hook {
                key: "1".to_string(),
                command: vec!["{{ field_1 }}".to_string(), "{{ field_2 }}".to_string()],
                r#if: None,
                optional: None,
                name: None,
                description: None,
            },
            Hook {
                key: "2".to_string(),
                command: vec!["echo".to_string(), "{{ project_name }}".to_string()],
                r#if: None,
                optional: None,
                name: None,
                description: None,
            },
        ];

        let results = run_hooks(
            &hooks,
            ".",
            &HashMap::from([
                ("field_1".to_string(), "echo".to_string()),
                ("field_2".to_string(), "out1".to_string()),
            ]),
            &HashMap::new(),
            None,
        )
        .expect("run_hooks failed, should have succeeded");

        assert!(
            results
                .iter()
                .all(|x| matches!(x, HookResult::Completed { .. })),
            "Expected all hooks to be completed, but got: {:?}",
            results
        );

        // Assert that hook 2 outputs "."
        assert!(
            results.iter().any(|x| match x {
                HookResult::Completed { hook, stdout, .. } if hook.key == "2" => {
                    stdout.trim() == "."
                }
                _ => false,
            }),
            "Hook 2 should output '.'"
        );
    }

    #[test]
    fn invalid_templated_cmd() {
        let hooks = vec![Hook {
            key: "1".to_string(),
            command: vec!["{{ field_1 }}".to_string(), "{{ field_2 }}".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        }];

        let results = run_hooks(
            &hooks,
            ".",
            &HashMap::from([("field_1".to_string(), "echo".to_string())]),
            &HashMap::new(),
            None,
        )
        .expect_err("run_hooks succeeded, should have failed");

        match results {
            Error::ErrorRenderingTemplate(_, _) => {}
            _ => panic!("Expected Error::ErrorRenderingTemplate, got {:?}", results),
        }
    }
}
