use std::{collections::HashMap, fs::remove_dir_all, path::PathBuf};

use spackle::core::{
    slot::{Slot, SlotType},
    template,
};
use tempdir::TempDir;

#[test]
fn fill_proj1() {
    let dir = TempDir::new("spackle").unwrap().into_path();

    let result = template::fill(
        &PathBuf::from("tests/data/proj1"),
        &dir,
        &HashMap::from([
            ("person_name".to_string(), "Joe Bloggs".to_string()),
            ("person_age".to_string(), "42".to_string()),
            ("file_name".to_string(), "main".to_string()),
        ]),
    );

    println!("{:?}", result);

    assert!(result.is_ok());

    remove_dir_all(&dir).unwrap();
}

#[test]
fn validate_dir_proj1() {
    let result = template::validate(
        &PathBuf::from("tests/data/proj1"),
        &vec![Slot {
            key: "defined_field".to_string(),
            r#type: SlotType::String,
            name: None,
            description: None,
        }],
    );

    assert!(result.is_err());
}

#[test]
fn validate_dir_proj2() {
    let result = template::validate(
        &PathBuf::from("tests/data/proj2"),
        &vec![Slot {
            key: "defined_field".to_string(),
            r#type: SlotType::String,
            name: None,
            description: None,
        }],
    );

    assert!(result.is_ok());
}
