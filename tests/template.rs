use std::{collections::HashMap, env::temp_dir, path::PathBuf};

use spackle::core::{
    slot::{Slot, SlotType},
    template,
};

#[test]
fn fill_proj1() {
    let dir = temp_dir();

    let result = template::fill(
        &PathBuf::from("tests/data/proj1"),
        HashMap::from([
            ("person_name".to_string(), "Joe Bloggs".to_string()),
            ("person_age".to_string(), "42".to_string()),
            ("file_name".to_string(), "main".to_string()),
        ]),
        &dir,
    );

    println!("{:?}", result);

    assert!(result.is_ok());
}

#[test]
fn validate_dir_proj1() {
    let result = template::validate(
        &PathBuf::from("tests/data/proj1"),
        &vec![Slot {
            key: "defined_field".to_string(),
            r#type: SlotType::String,
            name: "Defined field".to_string(),
            description: "Defined field".to_string(),
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
            name: "Defined field".to_string(),
            description: "Defined field".to_string(),
        }],
    );

    assert!(result.is_ok());
}
