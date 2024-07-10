use std::fs;

use spackle::core::config;
use tempdir::TempDir;

#[test]
fn load_empty() {
    let dir = TempDir::new("spackle").unwrap().into_path();
    fs::write(&dir.join("spackle.toml"), "").unwrap();

    let result = config::load(&dir);

    assert!(result.is_ok());
}
