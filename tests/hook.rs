use std::time::Instant;

use futures::executor;
use futures::pin_mut;
use futures::StreamExt;
use spackle::core::{config::Hook, hook};

#[test]
fn sleep() {
    let hooks = vec![
        Hook {
            name: "sleep 1ms".to_string(),
            command: vec!["sleep".to_string(), "0.001".to_string()],
        },
        Hook {
            name: "sleep 1ms".to_string(),
            command: vec!["sleep".to_string(), "0.001".to_string()],
        },
    ];

    let result = hook::run_hooks_async(hooks);

    assert!(result.is_ok());

    let stream = result.unwrap();

    pin_mut!(stream);

    let start_time = Instant::now();

    while let Some(status) = executor::block_on(stream.next()) {
        match status {
            hook::StreamStatus::HookCompleted(hook) => {
                println!("hook completed: {:?}", hook);
            }
            hook::StreamStatus::HookFailed(hook) => {
                panic!("hook failed: {:?}", hook);
            }
            hook::StreamStatus::Done => break,
        }
    }

    println!("time taken: {:?}", start_time.elapsed());
}
