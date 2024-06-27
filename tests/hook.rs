use futures::executor;
use futures::pin_mut;
use futures::StreamExt;
use spackle::core::{config::Hook, hook};

#[test]
fn sleep() {
    let hooks = vec![
        Hook {
            name: "sleep 1".to_string(),
            command: vec!["sleep".to_string(), "1".to_string()],
        },
        Hook {
            name: "sleep 2".to_string(),
            command: vec!["sleep".to_string(), "2".to_string()],
        },
    ];

    let result = hook::run_hooks_async(hooks);

    assert!(result.is_ok());

    let stream = result.unwrap();

    pin_mut!(stream);

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
}
