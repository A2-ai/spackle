use spackle::core::{config::Hook, hook};
use std::time::Instant;

#[test]
fn good_hooks() {
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

    let start_time = Instant::now();

    assert!(hook::run_hooks(hooks).is_ok());

    println!("time taken: {:?}", start_time.elapsed());
}

#[test]
fn bad_hook() {
    let hooks = vec![
        Hook {
            name: "sleep 1ms".to_string(),
            command: vec!["sleep".to_string(), "0.001".to_string()],
        },
        Hook {
            name: "exit 1".to_string(),
            command: vec!["exit".to_string(), "1".to_string()],
        },
    ];

    assert!(hook::run_hooks(hooks).is_err());
}
