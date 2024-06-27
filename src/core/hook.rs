use std::path::Path;

use async_process::Command;
use futures::{executor, pin_mut, stream, Stream, StreamExt};

use crate::core::hook;

use super::config::Hook;

pub enum CommandResult {
    HookCompleted(Hook),
    HookFailed { hook: Hook, stderr: String },
    Done,
}

#[derive(Debug)]
pub enum Error {
    ErrorSpawning(Hook, Box<dyn std::error::Error>),
    ErrorExecuting(Hook, String),
}

/// Run a set of hooks asynchronously and returns a stream of their execution results.
///
/// The `dir` argument is the directory to run the hooks in.
pub fn run_hooks_async(
    hooks: Vec<Hook>,
    dir: impl AsRef<Path>,
) -> Result<impl Stream<Item = CommandResult>, Error> {
    let mut children = Vec::new();
    for hook in hooks {
        let child = Command::new(&hook.command[0])
            .args(&hook.command[1..])
            .current_dir(dir.as_ref())
            .spawn();

        match child {
            Ok(child) => children.push((hook, child)),
            Err(e) => {
                return Err(Error::ErrorSpawning(hook, Box::new(e)));
            }
        }
    }

    let stream = stream::unfold(children.into_iter(), |mut children| async move {
        match children.next() {
            Some((hook, child)) => {
                let output = match child.output().await {
                    Ok(output) => output,
                    Err(_) => {
                        return Some((
                            CommandResult::HookFailed {
                                hook,
                                stderr: "".to_string(),
                            },
                            children,
                        ));
                    }
                };

                let result = match output.status.success() {
                    true => CommandResult::HookCompleted(hook),
                    false => CommandResult::HookFailed {
                        hook,
                        stderr: String::from_utf8_lossy(&output.stderr).into(),
                    },
                };

                Some((result, children))
            }
            None => None,
        }
    });

    Ok(stream)
}

/// Run a set of hooks, returning an error if any of the hooks fail.
///
/// The `dir` argument is the directory to run the hooks in.
pub fn run_hooks(hooks: Vec<Hook>, dir: impl AsRef<Path>) -> Result<(), Error> {
    let stream = run_hooks_async(hooks, dir)?;
    pin_mut!(stream);

    while let Some(status) = executor::block_on(stream.next()) {
        match status {
            hook::CommandResult::HookCompleted(hook) => {
                println!("hook completed: {:?}", hook);
            }
            hook::CommandResult::HookFailed { hook, stderr } => {
                return Err(Error::ErrorExecuting(hook, stderr));
            }
            hook::CommandResult::Done => break,
        }
    }

    Ok(())
}
