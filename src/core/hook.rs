use std::error::Error;

use async_process::Command;
use futures::{stream, Stream};

use super::config::Hook;

pub enum StreamStatus {
    HookCompleted(Hook),
    HookFailed(Hook),
    Done,
}

/// Run a set of hooks asynchronously and returns a stream of their execution results.
pub fn run_hooks_async(
    hooks: Vec<Hook>,
) -> Result<impl Stream<Item = StreamStatus>, Box<dyn Error>> {
    let mut children = Vec::new();
    for hook in hooks {
        let child = Command::new(&hook.command[0])
            .args(&hook.command[1..])
            .spawn();

        match child {
            Ok(child) => children.push((hook, child)),
            Err(e) => return Err(Box::new(e)),
        }
    }

    let stream = stream::unfold(children.into_iter(), |mut children| async move {
        match children.next() {
            Some((hook, child)) => {
                let status = match child.output().await {
                    Ok(output) => output.status,
                    Err(_) => {
                        // yield error
                        return Some((StreamStatus::HookFailed(hook), children));
                    }
                };

                let result = match status.success() {
                    true => StreamStatus::HookCompleted(hook),
                    false => StreamStatus::HookFailed(hook),
                };

                Some((result, children))
            }
            None => None,
        }
    });

    Ok(stream)
}
