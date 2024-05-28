use std::{collections::HashMap, path::PathBuf};

use spackle::core::fill;

#[test]
fn test_fill_proj1() {
    let result = fill::fill(
        &PathBuf::from("tests/data/proj1"),
        HashMap::from([
            ("person_name".to_string(), "Joe Bloggs".to_string()),
            ("person_age".to_string(), "42".to_string()),
            ("file_name".to_string(), "main".to_string()),
        ]),
        &PathBuf::from("tests/data/proj1-out"),
    );

    println!("{:?}", result);

    assert!(result.is_ok());
}
